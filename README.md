# metrics-exporter-influx


![build-badge][] [![downloads-badge][] ![release-badge][]][crate] [![docs-badge][]][docs] [![license-badge][]](#license)

[build-badge]: https://img.shields.io/github/actions/workflow/status/sevco/metrics-exporter-influx/ci.yml?branch=main
[downloads-badge]: https://img.shields.io/crates/d/metrics-exporter-influx.svg
[release-badge]: https://img.shields.io/crates/v/metrics-exporter-influx.svg
[license-badge]: https://img.shields.io/crates/l/metrics-exporter-influx.svg
[docs-badge]: https://docs.rs/metrics-exporter-influx/badge.svg
[crate]: https://crates.io/crates/metrics-exporter-influx
[docs]: https://docs.rs/metrics-exporter-influx


### Metrics reporter for https://github.com/metrics-rs/metrics that writes to InfluxDB.

## Usage

### Configuration

### Writing to a stderr

```rust
use std::time::Duration;

#[tokio::main]
async fn main() {
    InfluxBuilder::new().with_duration(Duration::from_secs(60)).install()?;
}
```

### Writing to a file
```rust
use std::fs::File;

#[tokio::main]
async fn main() {
    InfluxBuilder::new()
        .with_writer(File::create("/tmp/out.metrics")?)
        .install()?;
}

```

### Writing to http

#### Influx

```rust
#[tokio::main]
async fn main() {
    InfluxBuilder::new()
        .with_influx_api(
            "http://localhost:8086",
            "db/rp",
            None,
            None,
            None,
            Some("ns".to_string())
        )
        .install()?;
}
```

#### Grafana Cloud

[Grafana Cloud](https://grafana.com/docs/grafana-cloud/data-configuration/metrics/metrics-influxdb/push-from-telegraf/) 
supports the Influx Line Protocol exported by this exporter.

```rust
#[tokio::main]
async fn main() {
    InfluxBuilder::new()
        .with_grafna_cloud_api(
            "https://https://influx-prod-03-prod-us-central-0.grafana.net/api/v1/push/influx/write",
            Some("username".to_string()),
            Some("key")
        )
        .install()?;
}
```

## Builder API

The exporter runs as a tokio task. A tokio runtime should already be active
(e.g. via `#[tokio::main]`) when calling `build()`, `build_and_spawn()`, or
`install()`. If no runtime is found, one will be created automatically, but
this is not the preferred path.

`InfluxBuilder` provides three ways to create a recorder and exporter:

| Method | Returns | Description |
|---|---|---|
| `build()` | `(InfluxRecorder, ExporterFuture)` | Caller spawns the exporter future, passes the join handle to `shutdown_handle_with_task()`, and calls `set_global_recorder()`. |
| `build_and_spawn()` | `(InfluxRecorder, InfluxShutdownHandle)` | Spawns the exporter internally. Caller calls `set_global_recorder()`. |
| `install()` | `InfluxShutdownHandle` | Spawns the exporter and sets the global recorder. The most common entry point. |

### Graceful Shutdown

Calling `close()` on an `InfluxShutdownHandle` signals the background exporter loop
to perform a final flush, then joins the task. This ensures no metrics are lost when
the process exits (e.g. receiving SIGTERM).

```rust
let handle = InfluxBuilder::new()
    .with_grafana_cloud_api(endpoint, username, password)?
    .install()?;

// ... application runs ...

handle.close(); // final flush + join
```

For `build()` callers who spawn the exporter themselves, wire the join handle back
in for a clean shutdown:

```rust
let (recorder, exporter) = InfluxBuilder::new()
    .with_grafana_cloud_api(endpoint, username, password)?
    .build()?;

let jh = tokio::spawn(exporter);
let shutdown = recorder.shutdown_handle_with_task(jh);

metrics::set_global_recorder(recorder).unwrap();

// ... application runs ...

shutdown.close(); // signal → final flush → join
```
