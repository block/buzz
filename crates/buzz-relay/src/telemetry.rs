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

use opentelemetry_sdk::{resource::EnvResourceDetector, trace::SdkTracerProvider, Resource};

/// Build the OTEL [`Resource`] shared by the trace and metric providers.
///
/// Strategy (priority order):
/// 1. `OTEL_SERVICE_NAME` env var (standard OTEL env, read by [`EnvResourceDetector`])
/// 2. `service.name` in `OTEL_RESOURCE_ATTRIBUTES` env var (also read by env detector)
/// 3. Hard-coded fallback `buzz-relay`
///
/// The env detector is run first; its attributes win on merge.  The fallback
/// resource only provides `service.name` when neither env var sets it, so
/// user-supplied values are always respected.
///
/// Both the tracer provider (traces) and the meter provider (metrics) receive
/// the same `Resource` instance so Datadog can correlate spans and metrics on
/// the same `service.name`.
pub fn service_resource() -> Resource {
    // Start with buzz-relay as the service.name fallback, then overlay the env
    // detector.  The builder's `with_detector` call passes the detector output
    // as the "other" in merge(), so env-supplied values (OTEL_SERVICE_NAME,
    // OTEL_RESOURCE_ATTRIBUTES) always win over the fallback.
    Resource::builder_empty()
        .with_service_name("buzz-relay")
        .with_detector(Box::new(EnvResourceDetector::new()))
        .build()
}

/// Build and install the OTEL tracer provider, returning it so the caller
/// can register a shutdown hook.
///
/// Returns `None` when `OTEL_EXPORTER_OTLP_ENDPOINT` is unset — in that
/// case the tracing subscriber stack is unchanged.
pub fn try_init_tracer(resource: Resource) -> Option<SdkTracerProvider> {
    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_err() {
        return None;
    }

    match opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()
    {
        Ok(exporter) => {
            let provider = SdkTracerProvider::builder()
                .with_resource(resource)
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
