//! Separate binary: must run without a tokio runtime on the thread.

use httpmock::{Method, MockServer};
use metrics::{Key, Level, Metadata, Recorder};
use metrics_exporter_influx::InfluxBuilder;
use std::time::Duration;

fn metadata() -> Metadata<'static> {
    Metadata::new(module_path!(), Level::INFO, None)
}

/// build_and_spawn() without a tokio runtime creates its own multi-thread
/// runtime (1 worker). close() signals the exporter via Notify, joins the
/// task, then the owned runtime is dropped.
#[test]
fn build_and_spawn_shutdown_flushes_without_runtime() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(Method::POST).matches(|req| match &req.body {
            Some(body) => {
                let s = String::from_utf8_lossy(body);
                s.contains("no_rt_bas_counter") && s.contains("no_rt_bas_gauge")
            }
            None => false,
        });
        then.status(204);
    });

    // No tokio runtime on this thread — build_and_spawn() will create its own.
    let (recorder, shutdown) = InfluxBuilder::new()
        .with_grafana_cloud_api(
            format!("http://{}", server.address()).as_str(),
            Some("u".into()),
            Some("p".into()),
        )
        .unwrap()
        .with_gzip(false)
        .with_duration(Duration::from_secs(3600))
        .build_and_spawn()
        .unwrap();

    // Give the background thread's runtime time to start the exporter loop
    std::thread::sleep(Duration::from_millis(50));

    let m = metadata();
    recorder
        .register_counter(&Key::from_name("no_rt_bas_counter"), &m)
        .increment(3);
    recorder
        .register_gauge(&Key::from_name("no_rt_bas_gauge"), &m)
        .set(7.0);

    assert_eq!(mock.hits(), 0, "periodic flush should not have fired");

    // close() notifies the loop → final flush → joins the background thread
    shutdown.close();

    assert_eq!(
        mock.hits(),
        1,
        "expected exactly one flush from the background loop's shutdown path"
    );
}
