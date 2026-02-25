use httpmock::{Method, MockServer};
use metrics::{counter, gauge, histogram};
use metrics_exporter_influx::{InfluxBuilder, MetricData};
use tracing_subscriber::EnvFilter;

#[tokio::test(flavor = "multi_thread")]
async fn write_influx() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let server = MockServer::start();

    let mock = server.mock(|when, then| {
        when.header("authorization", "Token user:password")
            .method(Method::POST)
            .query_param("bucket", "db/rp")
            .query_param("org", "org_id")
            .query_param("precision", "ns")
            .matches(|request| match &request.body {
                Some(body) => {
                    let expected = [
                        "counter,tag0=value0,tag1=value1,tag2=value2,tag3=value3 field0=false,field1=\"0\",value=0i",
                        "counter,tag0=value0,tag1=value1,tag2=value2,tag3=value3 field0=false,field1=\"0\",value=2i",
                        "gauge,tag0=value0 field0=false,value=-1000",
                        "histogram,tag0=value0 count=100i,field0=false,max=99,min=0,p50=49.00390593892515,p90=89.00566416071958,p95=94.00049142147152,p99=97.99338832106014,p999=97.99338832106014,sum=4950"
                    ];
                    let content = String::from_utf8_lossy(body);
                    for (index, line) in content.lines().enumerate() {
                        match expected.get(index) {
                            Some(e) => {
                                if !line.starts_with(e) {
                                    return false
                                }
                            }
                            _ => return false
                        }
                    }
                    true
                }
                _ => false,
            });
        then.status(200);
    });

    let handle = InfluxBuilder::new()
        .with_influx_api(
            format!("http://{}", server.address()).as_str(),
            "db/rp".to_string(),
            Some("user".to_string()),
            Some("password".to_string()),
            Some("org_id".to_string()),
        )?
        .with_gzip(false)
        .add_global_tag("tag0", "value0")
        .add_global_field("field0", MetricData::Boolean(false))
        .install()?;

    counter!(
        "counter",
        "tag1" => "value1",
        "tag2" => "value2",
        "tag:tag3" => "value3",
        "field:field1" => "0",
    )
    .increment(2);

    gauge!("gauge").set(-1000.0);

    for i in 0..100 {
        histogram!("histogram").record(i as f64);
    }

    handle.close();

    mock.assert();
    Ok(())
}
