use httpmock::{Method, MockServer};
use metrics::{Key, Level, Metadata, Recorder};
use metrics_exporter_influx::InfluxBuilder;
use std::time::Duration;

fn metadata() -> Metadata<'static> {
    Metadata::new(module_path!(), Level::INFO, None)
}

fn grafana_builder(server: &MockServer) -> InfluxBuilder {
    InfluxBuilder::new()
        .with_grafana_cloud_api(
            format!("http://{}", server.address()).as_str(),
            Some("u".into()),
            Some("p".into()),
        )
        .unwrap()
        .with_gzip(false)
}

/// Shutdown handle flushes metrics that were never flushed by a periodic tick.
/// Simulates: Lambda receives SIGTERM before any exporter tick fires.
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_flushes_pending_metrics() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(Method::POST).matches(|req| match &req.body {
            Some(body) => {
                let s = String::from_utf8_lossy(body);
                s.contains("pending_counter") && s.contains("pending_gauge")
            }
            None => false,
        });
        then.status(204);
    });

    // Very long interval — periodic flush will never fire during this test
    let recorder = grafana_builder(&server)
        .with_duration(Duration::from_secs(3600))
        .build_recorder();
    let shutdown = recorder.shutdown_handle();
    let m = metadata();

    recorder
        .register_counter(&Key::from_name("pending_counter"), &m)
        .increment(7);
    recorder
        .register_gauge(&Key::from_name("pending_gauge"), &m)
        .set(3.14);

    shutdown.close();
    mock.assert();
}

/// Shutdown handle works even when the exporter future was never spawned.
/// The handle creates its own exporter instance for the final flush.
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_works_without_spawned_exporter() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(Method::POST).matches(|req| match &req.body {
            Some(body) => String::from_utf8_lossy(body).contains("orphan_gauge"),
            None => false,
        });
        then.status(204);
    });

    let (recorder, _exporter_not_spawned) = grafana_builder(&server)
        .with_duration(Duration::from_secs(3600))
        .build()
        .unwrap();
    let shutdown = recorder.shutdown_handle();

    recorder
        .register_gauge(&Key::from_name("orphan_gauge"), &metadata())
        .set(99.0);

    // Exporter future is dropped without spawning — shutdown must still flush
    drop(_exporter_not_spawned);
    shutdown.close();
    mock.assert();
}

/// Data written between the last periodic flush and shutdown is not lost.
/// Simulates: Lambda processes a request (writes metrics), then gets SIGTERM
/// before the next exporter tick.
#[tokio::test(flavor = "multi_thread")]
async fn shutdown_captures_late_writes() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(Method::POST);
        then.status(204);
    });

    let (recorder, exporter) = grafana_builder(&server)
        .with_duration(Duration::from_millis(100))
        .build()
        .unwrap();
    let shutdown = recorder.shutdown_handle();
    let m = metadata();

    // Initial data — will be picked up by the periodic flush
    recorder
        .register_counter(&Key::from_name("early_counter"), &m)
        .increment(1);

    // Spawn exporter and let at least one periodic tick fire
    tokio::spawn(exporter);
    tokio::time::sleep(Duration::from_millis(250)).await;

    let hits_after_periodic = mock.hits();
    assert!(
        hits_after_periodic >= 1,
        "expected at least one periodic flush, got {hits_after_periodic} hits"
    );

    // Write NEW data after the periodic flush has already fired
    recorder
        .register_gauge(&Key::from_name("late_gauge"), &m)
        .set(42.0);

    // Shutdown flush must capture the late-written gauge
    shutdown.close();
    assert!(
        mock.hits() > hits_after_periodic,
        "shutdown should trigger an additional flush beyond the {} periodic hit(s)",
        hits_after_periodic
    );
}
