use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    thread,
    time::Duration,
};

use metrics::{describe_gauge, gauge};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use metrics_process::Collector;
use strum::{EnumMessage, IntoEnumIterator};
use tracing::warn;

/// Trait for metrics gauge that provides common functionality.
pub trait MetricsGauge: Sized + IntoEnumIterator + EnumMessage + ToString {
    /// Describe the gauge metric.
    fn describe(&self) {
        describe_gauge!(self.to_string(), self.get_message().unwrap());
    }

    /// Set the gauge value.
    fn set(&self, value: f64) {
        gauge!(self.to_string()).set(value);
    }

    /// Increment the gauge value.
    fn increment(&self, value: f64) {
        gauge!(self.to_string()).increment(value);
    }

    /// Register all gauges.
    fn register_all() {
        for metric in Self::iter() {
            metric.describe();
        }
    }

    /// Initialize all gauges to 0.0.
    fn init_all() {
        for metric in Self::iter() {
            metric.set(0.0);
        }
    }
}

pub fn init_metrics(port: &u16) {
    init_metrics_with_buckets(port, &[]);
}

/// [MANTLE] Like [`init_metrics`], but renders the named histograms as Prometheus histograms
/// with the given bucket bounds (seconds, ascending). A histogram without an entry here is
/// exported as a summary, which `histogram_quantile` cannot aggregate.
pub fn init_metrics_with_buckets(port: &u16, histogram_buckets: &[(&str, &[f64])]) {
    let builder = prometheus_builder(histogram_buckets).with_http_listener(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        port.to_owned(),
    ));

    if let Err(e) = builder.install() {
        warn!("Failed to start metrics server: {}. Will continue without metrics.", e);
    }

    // Spawn a thread to collect process metrics.
    thread::spawn(move || {
        let collector = Collector::default();
        collector.describe();
        loop {
            // Periodically call `collect()` method to update information.
            collector.collect();
            thread::sleep(Duration::from_millis(750));
        }
    });
}

/// [MANTLE] The exporter configuration [`init_metrics_with_buckets`] installs, minus the HTTP
/// listener — separate so tests can render exactly what production renders.
pub fn prometheus_builder(histogram_buckets: &[(&str, &[f64])]) -> PrometheusBuilder {
    let mut builder = PrometheusBuilder::new();
    for (name, buckets) in histogram_buckets {
        // `set_buckets_for_metric` rejects only an empty list; skip one rather than refuse to
        // start — that metric then falls back to a summary.
        if buckets.is_empty() {
            warn!("Empty histogram buckets for {name}; it will be exported as a summary.");
            continue;
        }
        builder = builder
            .set_buckets_for_metric(Matcher::Full((*name).to_string()), buckets)
            .expect("a non-empty bucket list is accepted");
    }
    builder
}
