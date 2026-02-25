// https://github.com/metrics-rs/metrics/blob/0193688dac4ca646dbe44620040c20b9abf9bf5e/metrics-exporter-prometheus/src/distribution.rsa
// Copyright (c) 2021 Metrics Contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use crate::distribution::Distribution;
use metrics::{atomics::AtomicU64, HistogramFn};
use metrics_util::storage::AtomicBucket;
use quanta::Instant;
use std::sync::Arc;

/// Atomic metric storage for the prometheus exporter.
pub struct AtomicStorage;

impl<K> metrics_util::registry::Storage<K> for AtomicStorage {
    type Counter = Arc<AtomicU64>;
    type Gauge = Arc<AtomicU64>;
    type Histogram = Arc<AtomicBucketInstant<f64>>;

    fn counter(&self, _: &K) -> Self::Counter {
        Arc::new(AtomicU64::new(0))
    }

    fn gauge(&self, _: &K) -> Self::Gauge {
        Arc::new(AtomicU64::new(0))
    }

    fn histogram(&self, _: &K) -> Self::Histogram {
        Arc::new(AtomicBucketInstant::new())
    }
}

/// An `AtomicBucket` newtype wrapper that tracks the time of value insertion.
pub struct AtomicBucketInstant<T> {
    inner: AtomicBucket<(T, Instant)>,
}

impl<T> AtomicBucketInstant<T> {
    fn new() -> AtomicBucketInstant<T> {
        Self {
            inner: AtomicBucket::new(),
        }
    }
}

impl AtomicBucketInstant<f64> {
    pub fn record_samples(&self, mut distribution: Distribution) -> Distribution {
        self.inner.data_with(|samples| {
            distribution.record_samples(samples);
        });
        distribution
    }
}

impl HistogramFn for AtomicBucketInstant<f64> {
    fn record(&self, value: f64) {
        let now = Instant::now();
        self.inner.push((value, now));
    }
}
