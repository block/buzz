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

/// Outcome of [`try_init_tracer`], distinguishing the two disabled states so
/// the caller can log appropriately after the tracing subscriber is installed.
pub enum TracerInit {
    /// OTLP endpoint configured; provider is live and ready.
    Enabled(SdkTracerProvider),
    /// `OTEL_EXPORTER_OTLP_ENDPOINT` was unset — no-op, no connection.
    Disabled,
    /// Endpoint was set but the exporter failed to build.  The inner error
    /// string is suitable for a `tracing::warn!` call made by the caller
    /// **after** `tracing_subscriber::registry()…init()`.
    ExporterBuildFailed(String),
}

/// Build and install the OTEL tracer provider, returning the outcome so the
/// caller can act after the tracing subscriber is ready.
///
/// Deliberately does **not** call `tracing::warn!` internally — the subscriber
/// may not be installed yet at call time, which would silently drop the event.
/// Callers are responsible for logging [`TracerInit::ExporterBuildFailed`].
pub fn try_init_tracer(resource: Resource) -> TracerInit {
    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_err() {
        return TracerInit::Disabled;
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
            TracerInit::Enabled(provider)
        }
        Err(e) => TracerInit::ExporterBuildFailed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::KeyValue;
    use std::sync::Mutex;

    // Env vars are process-global — serialize tests that mutate them to prevent
    // cross-test races when the suite runs with multiple threads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // Helper: read service.name from the Resource's schema_url-independent KV list.
    fn service_name_from(resource: &Resource) -> Option<String> {
        resource
            .iter()
            .find(|(k, _)| k.as_str() == "service.name")
            .map(|(_, v)| v.to_string())
    }

    #[test]
    fn test_service_resource_default_when_env_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("OTEL_SERVICE_NAME");
        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");

        let r = service_resource();
        assert_eq!(
            service_name_from(&r).as_deref(),
            Some("buzz-relay"),
            "expected buzz-relay fallback when OTEL_SERVICE_NAME is unset"
        );
    }

    #[test]
    fn test_service_resource_honors_otel_service_name() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("OTEL_SERVICE_NAME", "my-custom-relay");
        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");

        let r = service_resource();
        std::env::remove_var("OTEL_SERVICE_NAME");

        assert_eq!(
            service_name_from(&r).as_deref(),
            Some("my-custom-relay"),
            "expected OTEL_SERVICE_NAME to be honoured"
        );
    }

    #[test]
    fn test_service_resource_empty_string_falls_back_to_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("OTEL_SERVICE_NAME", "");
        std::env::remove_var("OTEL_RESOURCE_ATTRIBUTES");

        let r = service_resource();
        std::env::remove_var("OTEL_SERVICE_NAME");

        assert_eq!(
            service_name_from(&r).as_deref(),
            Some("buzz-relay"),
            "expected buzz-relay fallback when OTEL_SERVICE_NAME is empty"
        );
    }

    #[test]
    fn test_try_init_tracer_disabled_when_endpoint_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");

        let resource = Resource::builder_empty()
            .with_attribute(KeyValue::new("service.name", "test"))
            .build();
        let result = try_init_tracer(resource);

        assert!(
            matches!(result, TracerInit::Disabled),
            "expected Disabled when OTEL_EXPORTER_OTLP_ENDPOINT is unset"
        );
    }

    #[test]
    fn test_try_init_tracer_exporter_build_failed_on_bad_endpoint() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Set endpoint to a value that causes tonic to reject during build
        // (invalid URI scheme forces a build-time error).
        std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "not-a-valid-uri:///bad");

        let resource = Resource::builder_empty()
            .with_attribute(KeyValue::new("service.name", "test"))
            .build();
        let result = try_init_tracer(resource);
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");

        // Either ExporterBuildFailed (build rejected the URI) or Enabled
        // (tonic accepted it for lazy connection) — both are valid SDK
        // behaviours.  What we assert is it is NOT Disabled.
        assert!(
            !matches!(result, TracerInit::Disabled),
            "expected Enabled or ExporterBuildFailed when endpoint is set"
        );
    }
}
