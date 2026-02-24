use httpmock::{Method, MockServer};
use metrics::{counter, gauge};
use metrics_exporter_influx::InfluxBuilder;
use std::time::Duration;

/// install() without a tokio runtime creates its own single-thread runtime on
/// a background thread via block_on_in_thread. close() exercises the Thread
/// join path: notify_one() → exporter final flush → thread::JoinHandle::join().
/// Exercises: close() → (Some(notify), ExporterJoinHandle::Thread(jh))
#[test]
fn install_shutdown_flushes_without_runtime() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(Method::POST).matches(|req| match &req.body {
            Some(body) => {
                let s = String::from_utf8_lossy(body);
                s.contains("no_rt_counter") && s.contains("no_rt_gauge")
            }
            None => false,
        });
        then.status(204);
    });

    // No tokio runtime on this thread — install() will create its own.
    let handle = InfluxBuilder::new()
        .with_grafana_cloud_api(
            format!("http://{}", server.address()).as_str(),
            Some("u".into()),
            Some("p".into()),
        )
        .unwrap()
        .with_gzip(false)
        .with_duration(Duration::from_secs(3600))
        .install()
        .unwrap();

    // Give the background thread's runtime time to start the exporter loop
    std::thread::sleep(Duration::from_millis(50));

    counter!("no_rt_counter").increment(3);
    gauge!("no_rt_gauge").set(7.0);

    assert_eq!(mock.hits(), 0, "periodic flush should not have fired");

    // close() notifies the loop → final flush → joins the background thread
    handle.close();

    assert_eq!(
        mock.hits(),
        1,
        "expected exactly one flush from the background loop's shutdown path"
    );
}
