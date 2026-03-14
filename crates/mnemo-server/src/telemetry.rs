//! OpenTelemetry integration for Mnemo.
//!
//! When `otel_enabled = true`, this module sets up a layered
//! `tracing_subscriber::Registry` with:
//!
//! 1. A `fmt` layer (human or JSON) for local console output.
//! 2. A `tracing-opentelemetry` layer that exports spans to an OTLP collector.
//!
//! When `otel_enabled = false`, only the `fmt` layer is installed (identical
//! to the original behaviour).

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::TracerProvider;
use opentelemetry_sdk::Resource;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::ObservabilitySection;

/// Initializes the global tracing subscriber.
///
/// Returns an optional `TracerProvider` that **must** be kept alive for the
/// duration of the process. Dropping the provider triggers a graceful flush
/// and shutdown of the OTLP exporter pipeline.
///
/// If OTel is enabled but the exporter fails to build (e.g. invalid endpoint),
/// this logs a warning and falls back to fmt-only tracing instead of panicking.
pub fn init_telemetry(obs: &ObservabilitySection) -> Option<TracerProvider> {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&obs.log_level));

    if obs.otel_enabled && !obs.otel_endpoint.is_empty() {
        // Build OTLP span exporter via tonic gRPC
        let exporter = match opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&obs.otel_endpoint)
            .build()
        {
            Ok(e) => e,
            Err(err) => {
                // Fall back to fmt-only tracing — do not crash the server.
                eprintln!(
                    "WARNING: Failed to build OTLP exporter (endpoint={:?}): {err}. \
                     Falling back to console-only tracing.",
                    obs.otel_endpoint
                );
                return init_fmt_only(obs, env_filter);
            }
        };

        let resource = Resource::new(vec![
            KeyValue::new("service.name", obs.otel_service_name.clone()),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION").to_string()),
        ]);

        let provider = TracerProvider::builder()
            .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
            .with_resource(resource)
            .build();

        let tracer = provider.tracer("mnemo");
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        // Layered subscriber: env_filter → otel → fmt
        // OTel layer is added before fmt to avoid type-mismatch between
        // json and pretty format layers.
        let registry = tracing_subscriber::registry()
            .with(env_filter)
            .with(otel_layer);

        if obs.log_format == "json" {
            registry
                .with(tracing_subscriber::fmt::layer().json())
                .init();
        } else {
            registry.with(tracing_subscriber::fmt::layer()).init();
        }

        tracing::info!(
            endpoint = %obs.otel_endpoint,
            service_name = %obs.otel_service_name,
            "OpenTelemetry OTLP tracing enabled"
        );

        Some(provider)
    } else {
        init_fmt_only(obs, env_filter)
    }
}

/// Install a fmt-only subscriber (no OTel). Used as the default path and
/// as the fallback when the OTLP exporter fails to build.
fn init_fmt_only(obs: &ObservabilitySection, env_filter: EnvFilter) -> Option<TracerProvider> {
    if obs.log_format == "json" {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }
    None
}

/// Gracefully shuts down the OpenTelemetry pipeline, flushing pending spans.
pub fn shutdown_telemetry(provider: Option<TracerProvider>) {
    if let Some(provider) = provider {
        if let Err(e) = provider.shutdown() {
            eprintln!("OpenTelemetry shutdown error: {e}");
        }
    }
}
