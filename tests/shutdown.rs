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
async fn build_recorder_shutdown_flushes_pending_metrics() {
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
        .set(3.0);

    shutdown.close();
    mock.assert();
}

/// Data written between the last periodic flush and shutdown is not lost.
/// Simulates: Lambda processes a request (writes metrics), then gets SIGTERM
/// before the next exporter tick.
/// Exercises: close() → (Some(notify), ExporterJoinHandle::Tokio(jh))
#[tokio::test(flavor = "multi_thread")]
async fn build_and_spawn_shutdown_captures_late_writes() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(Method::POST);
        then.status(204);
    });

    let (recorder, shutdown) = grafana_builder(&server)
        .with_duration(Duration::from_millis(100))
        .build_and_spawn()
        .unwrap();
    let m = metadata();

    // Initial data — will be picked up by the periodic flush
    recorder
        .register_counter(&Key::from_name("early_counter"), &m)
        .increment(1);

    // Let at least one periodic tick fire
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

/// build() returns a recorder and exporter future. The caller spawns the future
/// and passes the join handle to shutdown_handle_with_task().
/// Exercises: close() → (Some(notify), ExporterJoinHandle::Task { .. })
#[tokio::test(flavor = "multi_thread")]
async fn build_shutdown_flushes_with_caller_spawned_exporter() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(Method::POST).matches(|req| match &req.body {
            Some(body) => {
                let s = String::from_utf8_lossy(body);
                s.contains("manual_counter") && s.contains("manual_gauge")
            }
            None => false,
        });
        then.status(204);
    });

    let (recorder, exporter) = grafana_builder(&server)
        .with_duration(Duration::from_secs(3600))
        .build()
        .unwrap();

    let jh = tokio::spawn(exporter);
    let shutdown = recorder.shutdown_handle_with_task(jh);

    let m = metadata();

    // Let the exporter task start its select! loop
    tokio::task::yield_now().await;

    recorder
        .register_counter(&Key::from_name("manual_counter"), &m)
        .increment(99);
    recorder
        .register_gauge(&Key::from_name("manual_gauge"), &m)
        .set(77.0);

    assert_eq!(mock.hits(), 0, "periodic flush should not have fired");

    // close() signals the background loop → final write → joins the task
    shutdown.close();

    assert_eq!(
        mock.hits(),
        1,
        "expected exactly one flush from the background loop's shutdown path"
    );
}

/// build_and_spawn() returns a fully-wired shutdown handle (Notify + JoinHandle).
/// close() signals the background loop which does the final flush, then joins.
/// Exercises: close() → (Some(notify), ExporterJoinHandle::Tokio(jh))
#[tokio::test(flavor = "multi_thread")]
async fn build_and_spawn_shutdown_flushes_pending_metrics() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(Method::POST).matches(|req| match &req.body {
            Some(body) => {
                let s = String::from_utf8_lossy(body);
                s.contains("build_counter") && s.contains("build_gauge")
            }
            None => false,
        });
        then.status(204);
    });

    let (recorder, shutdown) = grafana_builder(&server)
        .with_duration(Duration::from_secs(3600))
        .build_and_spawn()
        .unwrap();
    let m = metadata();

    // Let the exporter task start its select! loop
    tokio::task::yield_now().await;

    recorder
        .register_counter(&Key::from_name("build_counter"), &m)
        .increment(10);
    recorder
        .register_gauge(&Key::from_name("build_gauge"), &m)
        .set(55.0);

    assert_eq!(mock.hits(), 0, "periodic flush should not have fired");

    // close() signals the background loop → final write → joins the task
    shutdown.close();

    assert_eq!(
        mock.hits(),
        1,
        "expected exactly one flush from the background loop's shutdown path"
    );
}
