//! OpenTelemetry tracing initialisation.
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────────┐
//! │  tracing crate (spans + events from #[instrument] and macros)      │
//! │          │                                                          │
//! │          ├── fmt::layer().json() → stdout  (always on)             │
//! │          └── OpenTelemetryLayer (only when endpoint env var set)   │
//! │                    ↓                                               │
//! │              SdkTracerProvider + OTLP batch exporter               │
//! │                    ↓                                               │
//! │              OTLP gRPC → collector / Datadog Agent                 │
//! └────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! When `OTEL_EXPORTER_OTLP_ENDPOINT` is **unset** this module is a no-op:
//! the JSON stdout logs continue to work exactly as before and no OTLP
//! connection is attempted.
//!
//! Standard OTEL env vars honoured automatically by the SDK:
//! - `OTEL_SERVICE_NAME` (default: `buzz-relay`)
//! - `OTEL_RESOURCE_ATTRIBUTES`
//! - `OTEL_TRACES_SAMPLER` (default: `parentbased_always_on`)
//! - `OTEL_TRACES_SAMPLER_ARG`

use opentelemetry_sdk::trace::SdkTracerProvider;

/// Build and install the OTEL tracer provider, returning it so the caller
/// can register a shutdown hook.
///
/// Returns `None` when `OTEL_EXPORTER_OTLP_ENDPOINT` is unset — in that
/// case the tracing subscriber stack is unchanged.
pub fn try_init_tracer() -> Option<SdkTracerProvider> {
    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_err() {
        return None;
    }

    match opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()
    {
        Ok(exporter) => {
            let provider = SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .build();
            opentelemetry::global::set_tracer_provider(provider.clone());
            Some(provider)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Failed to build OTLP trace exporter; distributed tracing disabled"
            );
            None
        }
    }
}
