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

use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use opentelemetry::trace::{SpanId, TraceContextExt as _, TraceId};
use opentelemetry_otlp::ExporterBuildError;
use opentelemetry_sdk::{resource::EnvResourceDetector, trace::SdkTracerProvider, Resource};
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    fmt::{
        format::{Format, FormatEvent, FormatFields, Json, Writer},
        FmtContext,
    },
    registry::LookupSpan,
    EnvFilter, Layer,
};

/// Captures the subscriber dispatch used to resolve tracing span IDs to their
/// OpenTelemetry contexts.
#[derive(Clone, Default)]
pub struct TraceContextLookup {
    dispatch: Arc<OnceLock<tracing::dispatcher::WeakDispatch>>,
}

impl TraceContextLookup {
    /// Build a JSON formatter backed by this subscriber dispatch lookup.
    pub fn json_formatter(&self, enabled: bool) -> TraceContextJson {
        TraceContextJson {
            inner: tracing_subscriber::fmt::format().json().flatten_event(true),
            enabled,
            context_lookup: self.clone(),
        }
    }

    fn nearest_otel_context(&self, span_id: &tracing::span::Id) -> Option<opentelemetry::Context> {
        let dispatch = self.dispatch.get()?.upgrade()?;
        let registry = dispatch.downcast_ref::<tracing_subscriber::Registry>()?;

        let context = registry.span(span_id)?.scope().find_map(|span| {
            let context = tracing_opentelemetry::get_otel_context(&span.id(), &dispatch)?;
            context.span().span_context().is_valid().then_some(context)
        });
        context
    }
}

impl<S: Subscriber> Layer<S> for TraceContextLookup {
    fn on_register_dispatch(&self, subscriber: &tracing::Dispatch) {
        let _ = self.dispatch.set(subscriber.downgrade());
    }
}

/// JSON event formatter that adds the active OpenTelemetry trace context.
///
/// Datadog recognizes the OpenTelemetry-standard `trace_id` and `span_id`
/// fields when they are lowercase hexadecimal strings. Events outside a valid
/// OpenTelemetry span retain the standard `tracing-subscriber` JSON format.
pub struct TraceContextJson {
    inner: Format<Json>,
    enabled: bool,
    context_lookup: TraceContextLookup,
}

struct CorrelationWriter<'writer> {
    inner: Writer<'writer>,
    trace_id: TraceId,
    span_id: SpanId,
    injected: bool,
}

impl fmt::Write for CorrelationWriter<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.injected {
            return self.inner.write_str(value);
        }

        let Some(object_start) = value.find('{') else {
            return self.inner.write_str(value);
        };
        self.inner.write_str(&value[..=object_start])?;
        write!(
            self.inner,
            "\"trace_id\":\"{}\",\"span_id\":\"{}\",",
            self.trace_id, self.span_id
        )?;
        self.injected = true;
        self.inner.write_str(&value[object_start + 1..])
    }
}

impl<S, N> FormatEvent<S, N> for TraceContextJson
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        if !self.enabled {
            return self.inner.format_event(ctx, writer, event);
        }

        let otel_context = match event.parent() {
            Some(span_id) => self.context_lookup.nearest_otel_context(span_id),
            None if event.is_contextual() => Some(opentelemetry::Context::current()),
            None => None,
        };
        let Some(otel_context) = otel_context else {
            return self.inner.format_event(ctx, writer, event);
        };
        let otel_span = otel_context.span();
        let span_context = otel_span.span_context();

        if !span_context.is_valid() {
            return self.inner.format_event(ctx, writer, event);
        }

        let trace_id = span_context.trace_id();
        let span_id = span_context.span_id();

        // Events may define fields with the correlation names themselves. In
        // that uncommon case, overwrite them rather than emitting duplicate
        // JSON keys. Preserve the allocation-free streaming path for ordinary
        // events.
        let fields = event.metadata().fields();
        if fields.field("trace_id").is_some() || fields.field("span_id").is_some() {
            let mut json = String::new();
            self.inner
                .format_event(ctx, Writer::new(&mut json), event)?;
            let mut object: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(json.trim_end()).map_err(|_| fmt::Error)?;
            object.insert("trace_id".into(), trace_id.to_string().into());
            object.insert("span_id".into(), span_id.to_string().into());
            writer.write_str(&serde_json::to_string(&object).map_err(|_| fmt::Error)?)?;
            return writeln!(writer);
        }

        let mut writer = CorrelationWriter {
            inner: writer,
            trace_id,
            span_id,
            injected: false,
        };
        self.inner
            .format_event(ctx, Writer::new(&mut writer), event)
    }
}

/// Build the filter for spans exported through OpenTelemetry.
///
/// This is intentionally independent from `RUST_LOG`: changing stdout log
/// verbosity must not remove parent spans from exported traces. Set
/// `BUZZ_OTEL_FILTER` to override the default targets.
pub fn otel_env_filter(configured: Option<&str>) -> EnvFilter {
    EnvFilter::new(configured.unwrap_or("buzz_relay=info,buzz_datastore=info"))
}

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

    let result = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build();
    classify_exporter_result(result, resource)
}

/// Map a `Result<SpanExporter, ExporterBuildError>` to a [`TracerInit`] variant.
///
/// Extracted so the `Err → ExporterBuildFailed` classification can be unit-tested
/// deterministically without relying on SDK behaviour for specific URI inputs.
fn classify_exporter_result(
    result: Result<opentelemetry_otlp::SpanExporter, ExporterBuildError>,
    resource: Resource,
) -> TracerInit {
    match result {
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
    use opentelemetry::{trace::TracerProvider as _, KeyValue};
    use opentelemetry_sdk::trace::InMemorySpanExporter;
    use std::{
        io,
        sync::{Arc, Mutex},
    };
    use tracing_subscriber::prelude::*;

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

    #[derive(Clone)]
    struct CapturingWriter(Arc<Mutex<Vec<u8>>>);

    impl io::Write for CapturingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn trace_context_json_correlates_nested_span_logs() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let output_writer = Arc::clone(&output);
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer = provider.tracer("trace-context-json-test");
        let context_lookup = TraceContextLookup::default();

        let subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .event_format(context_lookup.json_formatter(true))
                    .with_writer(move || CapturingWriter(Arc::clone(&output_writer)))
                    .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                        metadata.target() != "stdout_filtered"
                    })),
            )
            .with(
                tracing_opentelemetry::layer()
                    .with_tracer(tracer)
                    .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                        !matches!(metadata.target(), "filtered" | "otel_event_filtered")
                    })),
            )
            .with(
                context_lookup
                    .clone()
                    .with_filter(tracing_subscriber::filter::LevelFilter::OFF),
            );

        tracing::subscriber::with_default(subscriber, || {
            let explicit = tracing::info_span!("explicit");
            let root = tracing::info_span!("root");
            root.in_scope(|| {
                tracing::info!(answer = 42, "root event");
                tracing::info!(
                    trace_id = "event-provided-trace",
                    span_id = "event-provided-span",
                    "colliding-fields event"
                );
                tracing::info!(parent: &explicit, "explicit-parent event");
                tracing::info!(parent: None, "explicit-root event");
                let child = tracing::info_span!("child");
                child.in_scope(|| tracing::info!("child event"));

                let filtered_child = tracing::info_span!(target: "filtered", "filtered-child");
                filtered_child.in_scope(|| tracing::info!("filtered-child event"));
                tracing::info!(
                    parent: &filtered_child,
                    "explicit-filtered-child event"
                );

                let stdout_filtered_child =
                    tracing::info_span!(target: "stdout_filtered", "stdout-filtered-child");
                stdout_filtered_child.in_scope(|| tracing::info!("stdout-filtered-child event"));

                tracing::info!(target: "otel_event_filtered", "otel-filtered event");
            });
            let filtered = tracing::info_span!(target: "filtered", "filtered");
            filtered.in_scope(|| tracing::info!("filtered-span event"));
            tracing::info!("unscoped event");
        });

        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        let root = spans.iter().find(|span| span.name == "root").unwrap();
        let explicit = spans.iter().find(|span| span.name == "explicit").unwrap();
        let child = spans.iter().find(|span| span.name == "child").unwrap();
        let stdout_filtered_child = spans
            .iter()
            .find(|span| span.name == "stdout-filtered-child")
            .unwrap();

        let bytes = output.lock().unwrap().clone();
        let output = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        let logs: Vec<serde_json::Value> = lines
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(logs.len(), 11);

        assert_eq!(logs[0]["message"], "root event");
        assert_eq!(logs[0]["answer"], 42);
        assert_eq!(
            logs[0]["trace_id"],
            root.span_context.trace_id().to_string()
        );
        assert_eq!(logs[0]["span_id"], root.span_context.span_id().to_string());
        assert_eq!(logs[0]["trace_id"].as_str().unwrap().len(), 32);
        assert_eq!(logs[0]["span_id"].as_str().unwrap().len(), 16);

        assert_eq!(logs[1]["message"], "colliding-fields event");
        assert_eq!(
            logs[1]["trace_id"],
            root.span_context.trace_id().to_string()
        );
        assert_eq!(logs[1]["span_id"], root.span_context.span_id().to_string());
        assert_eq!(lines[1].matches("\"trace_id\":").count(), 1);
        assert_eq!(lines[1].matches("\"span_id\":").count(), 1);

        assert_eq!(logs[2]["message"], "explicit-parent event");
        assert_eq!(
            logs[2]["trace_id"],
            explicit.span_context.trace_id().to_string()
        );
        assert_eq!(
            logs[2]["span_id"],
            explicit.span_context.span_id().to_string()
        );

        assert_eq!(logs[3]["message"], "explicit-root event");
        assert!(logs[3].get("trace_id").is_none());
        assert!(logs[3].get("span_id").is_none());

        assert_eq!(logs[4]["message"], "child event");
        assert_eq!(
            logs[4]["trace_id"],
            child.span_context.trace_id().to_string()
        );
        assert_eq!(logs[4]["span_id"], child.span_context.span_id().to_string());
        assert_eq!(logs[0]["trace_id"], logs[4]["trace_id"]);

        assert_eq!(logs[5]["message"], "filtered-child event");
        assert_eq!(
            logs[5]["trace_id"],
            root.span_context.trace_id().to_string()
        );
        assert_eq!(logs[5]["span_id"], root.span_context.span_id().to_string());

        assert_eq!(logs[6]["message"], "explicit-filtered-child event");
        assert_eq!(
            logs[6]["trace_id"],
            root.span_context.trace_id().to_string()
        );
        assert_eq!(logs[6]["span_id"], root.span_context.span_id().to_string());

        assert_eq!(logs[7]["message"], "stdout-filtered-child event");
        assert_eq!(
            logs[7]["trace_id"],
            stdout_filtered_child.span_context.trace_id().to_string()
        );
        assert_eq!(
            logs[7]["span_id"],
            stdout_filtered_child.span_context.span_id().to_string()
        );

        assert_eq!(logs[8]["message"], "otel-filtered event");
        assert_eq!(
            logs[8]["trace_id"],
            root.span_context.trace_id().to_string()
        );
        assert_eq!(logs[8]["span_id"], root.span_context.span_id().to_string());

        assert_eq!(logs[9]["message"], "filtered-span event");
        assert!(logs[9].get("trace_id").is_none());
        assert!(logs[9].get("span_id").is_none());

        assert_eq!(logs[10]["message"], "unscoped event");
        assert!(logs[10].get("trace_id").is_none());
        assert!(logs[10].get("span_id").is_none());
    }

    #[test]
    fn trace_context_lookup_does_not_enable_callsites() {
        let context_lookup = TraceContextLookup::default();
        let subscriber = tracing_subscriber::registry().with(
            context_lookup
                .clone()
                .with_filter(tracing_subscriber::filter::LevelFilter::OFF),
        );

        tracing::subscriber::with_default(subscriber, || {
            assert!(context_lookup
                .dispatch
                .get()
                .and_then(tracing::dispatcher::WeakDispatch::upgrade)
                .is_some());
            assert!(!tracing::enabled!(
                target: "trace_context_lookup_filter_test",
                tracing::Level::ERROR
            ));
        });
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

    /// Pin the `Err(ExporterBuildError) → TracerInit::ExporterBuildFailed` mapping
    /// deterministically by synthesising an error value directly, without relying on
    /// SDK/tonic behaviour for a particular URI at build time.
    #[test]
    fn test_classify_exporter_result_maps_err_to_exporter_build_failed() {
        let resource = Resource::builder_empty()
            .with_attribute(KeyValue::new("service.name", "test"))
            .build();

        // Construct a concrete ExporterBuildError variant directly.
        // InvalidUri is available under the grpc-tonic feature we already depend on.
        let err = ExporterBuildError::InvalidUri(
            "bad-scheme://host".to_string(),
            "unsupported scheme".to_string(),
        );
        let result = classify_exporter_result(Err(err), resource);

        assert!(
            matches!(result, TracerInit::ExporterBuildFailed(_)),
            "expected ExporterBuildFailed when classify_exporter_result receives an Err"
        );
    }
}
