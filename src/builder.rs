use crate::data::MetricData;
use crate::distribution::DistributionBuilder;
#[cfg(feature = "http")]
use crate::http::APIVersion;
use crate::matcher::Matcher;
use crate::recorder::{ExporterConfig, HttpConfig, InfluxRecorder, InfluxShutdownHandle, Inner};
use crate::registry::AtomicStorage;
use metrics_util::registry::Registry;
use metrics_util::{parse_quantiles, Quantile};
#[cfg(feature = "http")]
use reqwest::Url;
use std::collections::HashMap;
use std::fmt::Display;
use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use std::{io, thread};
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::{runtime, time};

pub type ExporterFuture = Pin<Box<dyn Future<Output = Result<(), anyhow::Error>> + Send + 'static>>;

#[derive(Debug, Error)]
pub enum BuildError {
    /// An invalid URL was supplied
    #[cfg(feature = "http")]
    #[error("invalid endpoint `{0}`")]
    InvalidEndpoint(String),
    /// There was an error in http communications
    #[cfg(feature = "http")]
    #[error("http error `{0}`")]
    HttpError(#[from] reqwest::Error),
    /// There was an issue when creating the necessary Tokio runtime to launch the exporter.
    #[error("failed to create Tokio runtime for exporter: {0}")]
    FailedToCreateRuntime(String),
    /// Installing the recorder did not succeed.
    #[error("failed to install exporter as global recorder: {0}")]
    FailedToSetGlobalRecorder(String),
    /// Empty buckets or quantiles
    #[error("empty buckets or quantiles")]
    EmptyBucketsOrQuantiles,
}

pub struct InfluxBuilder {
    pub(crate) exporter_config: ExporterConfig,
    pub(crate) duration: Option<Duration>,
    pub(crate) global_tags: Option<HashMap<String, String>>,
    pub(crate) global_fields: Option<HashMap<String, MetricData>>,
    pub(crate) quantiles: Vec<Quantile>,
    pub(crate) buckets: Option<Vec<f64>>,
    pub(crate) bucket_overrides: Option<HashMap<Matcher, Vec<f64>>>,
}

impl InfluxBuilder {
    pub fn new() -> Self {
        let quantiles = parse_quantiles(&[0.0, 0.5, 0.9, 0.95, 0.99, 0.999, 1.0]);
        Self {
            exporter_config: ExporterConfig::File(Arc::new(Mutex::new(io::stderr()))),
            global_tags: None,
            duration: None,
            global_fields: None,
            quantiles,
            buckets: None,
            bucket_overrides: None,
        }
    }

    pub fn with_quantiles(mut self, quantiles: &[f64]) -> Result<Self, BuildError> {
        if quantiles.is_empty() {
            Err(BuildError::EmptyBucketsOrQuantiles)
        } else {
            self.quantiles = parse_quantiles(quantiles);
            Ok(self)
        }
    }

    pub fn with_buckets(mut self, values: &[f64]) -> Result<Self, BuildError> {
        if values.is_empty() {
            Err(BuildError::EmptyBucketsOrQuantiles)
        } else {
            self.buckets = Some(values.to_vec());
            Ok(self)
        }
    }

    pub fn add_buckets_for_metric(
        mut self,
        matcher: Matcher,
        values: &[f64],
    ) -> Result<Self, BuildError> {
        if values.is_empty() {
            Err(BuildError::EmptyBucketsOrQuantiles)
        } else {
            self.bucket_overrides
                .get_or_insert_with(HashMap::new)
                .entry(matcher)
                .or_insert(values.to_vec());
            self.buckets = Some(values.to_vec());
            Ok(self)
        }
    }

    pub fn add_global_tag<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        if let Some(tags) = &mut self.global_tags {
            tags.insert(key.into(), value.into());
        } else {
            self.global_tags = Some(vec![(key.into(), value.into())].into_iter().collect());
        }
        self
    }

    pub fn add_global_field<K: Into<String>>(mut self, key: K, value: MetricData) -> Self {
        if let Some(fields) = &mut self.global_fields {
            fields.insert(key.into(), value);
        } else {
            self.global_fields = Some(vec![(key.into(), value)].into_iter().collect());
        }
        self
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    #[cfg(feature = "http")]
    pub fn with_influx_api<E>(
        mut self,
        endpoint: E,
        bucket: String,
        username: Option<String>,
        password: Option<String>,
        org: Option<String>,
    ) -> Result<Self, BuildError>
    where
        Url: TryFrom<E>,
        <Url as TryFrom<E>>::Error: Display,
    {
        self.exporter_config = ExporterConfig::Http(Arc::new(HttpConfig {
            api_version: APIVersion::Influx {
                bucket,
                precision: Some("ns".to_string()),
                org,
            },
            gzip: true,
            endpoint: Url::try_from(endpoint)
                .map_err(|e| BuildError::InvalidEndpoint(e.to_string()))?,
            username,
            password,
        }));
        Ok(self)
    }

    #[cfg(feature = "http")]
    pub fn with_gzip(mut self, gzip: bool) -> Self {
        self.exporter_config = match self.exporter_config {
            ExporterConfig::Http(http) => ExporterConfig::Http(Arc::new(HttpConfig {
                gzip,
                ..(*http).to_owned()
            })),
            config => config,
        };
        self
    }

    #[cfg(feature = "http")]
    pub fn with_grafana_cloud_api<E>(
        mut self,
        endpoint: E,
        username: Option<String>,
        password: Option<String>,
    ) -> Result<Self, BuildError>
    where
        Url: TryFrom<E>,
        <Url as TryFrom<E>>::Error: Display,
    {
        self.exporter_config = ExporterConfig::Http(Arc::new(HttpConfig {
            api_version: APIVersion::GrafanaCloud,
            gzip: true,
            endpoint: Url::try_from(endpoint)
                .map_err(|e| BuildError::InvalidEndpoint(e.to_string()))?,
            username,
            password,
        }));
        Ok(self)
    }

    pub fn with_writer<W: Write + Send + Sync + 'static>(mut self, writer: W) -> Self {
        self.exporter_config = ExporterConfig::File(Arc::new(Mutex::new(writer)));
        self
    }

    pub fn build_recorder(self) -> InfluxRecorder {
        InfluxRecorder::new(
            Arc::new(Inner {
                registry: Registry::new(AtomicStorage),
                global_tags: self.global_tags.unwrap_or_default(),
                global_fields: self.global_fields.unwrap_or_default(),
                distribution_builder: DistributionBuilder::new(
                    self.quantiles,
                    self.buckets,
                    self.bucket_overrides,
                ),
                counter_registrations: Default::default(),
            }),
            self.exporter_config,
        )
    }

    pub fn build(self) -> Result<(InfluxRecorder, ExporterFuture), BuildError> {
        let interval = time::interval(self.duration.unwrap_or(Duration::from_secs(10)));
        let recorder = self.build_recorder();
        let mut exporter = recorder.exporter()?;
        let exporter_future = Box::pin(async move { exporter.run(interval).await });
        Ok((recorder, exporter_future))
    }

    pub fn install(self) -> Result<InfluxShutdownHandle, BuildError> {
        let recorder = if let Ok(handle) = runtime::Handle::try_current() {
            let (recorder, exporter) = {
                let _g = handle.enter();
                self.build()?
            };
            handle.spawn(exporter);
            recorder
        } else {
            let thread_name = format!(
                "metrics-exporter-influx-{}",
                self.exporter_config.as_type_str()
            );

            let runtime = runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| BuildError::FailedToCreateRuntime(e.to_string()))?;

            let (recorder, exporter) = {
                let _g = runtime.enter();
                self.build()?
            };

            thread::Builder::new()
                .name(thread_name)
                .spawn(move || runtime.block_on(exporter))
                .map_err(|e| BuildError::FailedToCreateRuntime(e.to_string()))?;

            recorder
        };

        let shutdown_handle = recorder.shutdown_handle();
        metrics::set_global_recorder(recorder)
            .map_err(|e| BuildError::FailedToSetGlobalRecorder(format!("{e:?}")))?;
        Ok(shutdown_handle)
    }
}

impl Default for InfluxBuilder {
    fn default() -> Self {
        InfluxBuilder::new()
    }
}
