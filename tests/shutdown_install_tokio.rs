//! Separate binary: install() calls set_global_recorder(), which is once-per-process.

use httpmock::{Method, MockServer};
use metrics::{counter, gauge};
use metrics_exporter_influx::InfluxBuilder;
use std::time::Duration;

/// install() on a multi-thread runtime wires Notify + JoinHandle into the
/// shutdown handle. close() signals the background loop via Notify, the loop
/// does one final write() and breaks, then close() joins the task.
#[tokio::test(flavor = "multi_thread")]
async fn install_shutdown_flushes_pending_metrics() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(Method::POST).matches(|req| match &req.body {
            Some(body) => {
                let s = String::from_utf8_lossy(body);
                s.contains("installed_counter") && s.contains("installed_gauge")
            }
            None => false,
        });
        then.status(204);
    });

    // Long interval — periodic flush will never fire during this test.
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

    // Let the exporter task start its select! loop
    tokio::task::yield_now().await;

    counter!("installed_counter").increment(5);
    gauge!("installed_gauge").set(42.0);

    assert_eq!(mock.hits(), 0, "periodic flush should not have fired");

    // close() notifies the loop → final write → joins the tokio task
    handle.close();

    assert_eq!(
        mock.hits(),
        1,
        "expected exactly one flush from the background loop's shutdown path"
    );
}
