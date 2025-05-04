use indicatif::ProgressStyle;
use std::sync::LazyLock;
use tracing::level_filters::LevelFilter;
use tracing_indicatif::{
    filter::IndicatifFilter, util::FilteredFormatFields, writer, IndicatifLayer, IndicatifWriter,
};
use tracing_subscriber::{
    layer::{Identity, SubscriberExt},
    util::SubscriberInitExt as _,
    EnvFilter, Layer, Registry,
};

pub static PB_PROGRESS_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template(
        "{span_child_prefix} {wide_msg} {bar:10} ({elapsed}) {pos:>7}/{len:7}",
    )
    .expect("invalid progress template")
});
pub static PB_TRANSFER_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template(
        "{span_child_prefix} {wide_msg} {binary_bytes:>7}/{binary_total_bytes:7}@{decimal_bytes_per_sec} ({elapsed}) {bar:10} "
    )
    .expect("invalid progress template")
});
pub static PB_SPINNER_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
    ProgressStyle::with_template(
        "{span_child_prefix}{spinner} {wide_msg} ({elapsed}) {pos:>7}/{len:7}",
    )
    .expect("invalid progress template")
});

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Init(#[from] tracing_subscriber::util::TryInitError),
}

#[derive(Clone)]
pub struct TracingHandle {
    stdout_writer: IndicatifWriter<writer::Stdout>,
    stderr_writer: IndicatifWriter<writer::Stderr>,
}

impl TracingHandle {
    /// Returns a writer for [std::io::Stdout] that ensures its output will not be clobbered by
    /// active progress bars.
    ///
    /// Instead of `println!(...)` prefer `writeln!(handle.get_stdout_writer(), ...)`
    pub fn get_stdout_writer(&self) -> IndicatifWriter<writer::Stdout> {
        // clone is fine here because its only a wrapper over an `Arc`
        self.stdout_writer.clone()
    }

    /// Returns a writer for [std::io::Stderr] that ensures its output will not be clobbered by
    /// active progress bars.
    ///
    /// Instead of `println!(...)` prefer `writeln!(handle.get_stderr_writer(), ...)`.
    pub fn get_stderr_writer(&self) -> IndicatifWriter<writer::Stderr> {
        // clone is fine here because its only a wrapper over an `Arc`
        self.stderr_writer.clone()
    }

    /// This will flush possible attached tracing providers, e.g. otlp exported, if enabled.
    /// If there is none enabled this will result in a noop.
    ///
    /// It will wait until the flush is complete.
    pub async fn flush(&self) -> Result<(), Error> {
        Ok(())
    }

    /// This will flush all attached tracing providers and will wait until the flush is completed, then call shutdown.
    /// If no tracing providers like otlp are attached then this will be a noop.
    ///
    /// This should only be called on a regular shutdown.
    pub async fn shutdown(&self) -> Result<(), Error> {
        self.flush().await?;
        Ok(())
    }
}

#[must_use = "Don't forget to call build() to enable tracing."]
#[derive(Default)]
pub struct TracingBuilder {
    progess_bar: bool,
}

impl TracingBuilder {
    /// Enable progress bar layer, default is disabled
    pub fn enable_progressbar(mut self) -> TracingBuilder {
        self.progess_bar = true;
        self
    }

    /// This will setup tracing based on the configuration passed in.
    /// It will setup a stderr writer output layer and configure EnvFilter to honor RUST_LOG.
    /// The EnvFilter will be applied to all configured layers, also otlp.
    ///
    /// It will also configure otlp if the feature is enabled and a service_name was provided. It
    /// will then correctly setup a channel which is later used for flushing the provider.
    pub fn build(self) -> Result<TracingHandle, Error> {
        self.build_with_additional(Identity::new())
    }

    /// Similar to `build()` but allows passing in an additional tracing [`Layer`].
    ///
    /// This method is generic over the `Layer` to avoid the runtime cost of dynamic dispatch.
    /// While it only allows passing a single `Layer`, it can be composed of multiple ones:
    ///
    /// ```ignore
    /// build_with_additional(
    ///   fmt::layer()
    ///     .and_then(some_other_layer)
    ///     .and_then(yet_another_layer)
    ///     .with_filter(my_filter)
    /// )
    /// ```
    /// [`Layer`]: tracing_subscriber::layer::Layer
    pub fn build_with_additional<L>(self, additional_layer: L) -> Result<TracingHandle, Error>
    where
        L: Layer<Registry> + Send + Sync + 'static,
    {
        // Set up the tracing subscriber.
        let indicatif_layer = IndicatifLayer::new().with_progress_style(PB_SPINNER_STYLE.clone());
        let stdout_writer = indicatif_layer.get_stdout_writer();
        let stderr_writer = indicatif_layer.get_stderr_writer();

        let layered = tracing_subscriber::fmt::Layer::new()
            .fmt_fields(FilteredFormatFields::new(
                tracing_subscriber::fmt::format::DefaultFields::new(),
                |field| field.name() != "indicatif.pb_show",
            ))
            .with_writer(indicatif_layer.get_stderr_writer())
            .compact()
            .and_then((self.progess_bar).then(|| {
                indicatif_layer.with_filter(
                    // only show progress for spans with indicatif.pb_show field being set
                    IndicatifFilter::new(false),
                )
            }));

        let layered = layered.with_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env()
                .expect("invalid RUST_LOG"),
        );

        tracing_subscriber::registry()
            // TODO: if additional_layer has global filters, there is a risk that it will disable the "default" ones,
            // while it could be solved by registering `additional_layer` last, it requires boxing `additional_layer`.
            .with(additional_layer)
            .with(layered)
            .try_init()?;

        Ok(TracingHandle {
            stdout_writer,
            stderr_writer,
        })
    }
}

// Metric export interval should be less than or equal to 15s
// if the metrics may be converted to Prometheus metrics.
// Prometheus' query engine and compatible implementations
// require ~4 data points / interval for range queries,
// so queries ranging over 1m requre <= 15s scrape intervals.
// OTEL SDKS also respect the env var `OTEL_METRIC_EXPORT_INTERVAL` (no underscore prefix).
const _OTEL_METRIC_EXPORT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
