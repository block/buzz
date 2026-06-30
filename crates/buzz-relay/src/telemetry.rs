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
//! Standard OTEL env vars honoured:
//! - `OTEL_SERVICE_NAME` (default: `buzz-relay`; read explicitly — not via SDK detector)
//! - `OTEL_RESOURCE_ATTRIBUTES` (overlaid by [`EnvResourceDetector`])
//! - `OTEL_TRACES_SAMPLER` (default: `parentbased_always_on`)
//! - `OTEL_TRACES_SAMPLER_ARG`

use opentelemetry_sdk::{resource::EnvResourceDetector, trace::SdkTracerProvider, Resource};

/// Build the OTEL [`Resource`] used by the trace provider.
///
/// Strategy (priority order):
/// 1. `service.name` in `OTEL_RESOURCE_ATTRIBUTES` — overlaid last by
///    [`EnvResourceDetector`], wins over everything below.
/// 2. `OTEL_SERVICE_NAME` — read explicitly (non-empty wins over the fallback).
/// 3. Hard-coded fallback `buzz-relay`.
///
/// Note: [`EnvResourceDetector`] only reads `OTEL_RESOURCE_ATTRIBUTES`; it
/// does **not** read `OTEL_SERVICE_NAME`.  `SdkProvidedResourceDetector` does
/// read `OTEL_SERVICE_NAME` but always emits a `service.name` key (falling
/// back to `unknown_service:<exe>` when unset), which would clobber our
/// `buzz-relay` default.  We therefore read `OTEL_SERVICE_NAME` explicitly
/// so the fallback is fully under our control.
///
/// The tracer provider receives this `Resource` so Datadog can identify
/// spans under the correct `service.name`.
pub fn service_resource() -> Resource {
    // Honor OTEL_SERVICE_NAME when set+non-empty; otherwise use buzz-relay.
    // EnvResourceDetector overlays OTEL_RESOURCE_ATTRIBUTES last, so an
    // explicit service.name there still wins over OTEL_SERVICE_NAME per spec.
    let service_name = std::env::var("OTEL_SERVICE_NAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "buzz-relay".to_string());

    Resource::builder_empty()
        .with_service_name(service_name)
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
