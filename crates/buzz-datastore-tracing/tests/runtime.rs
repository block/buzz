use buzz_datastore_tracing::datastore_span;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use opentelemetry::trace::{SpanKind, Status, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

const DIRECT_ERROR: &str = "raw-secret-direct-error";
const QUESTION_ERROR: &str = "raw-secret-question-error";

fn question_path(fail: bool) -> Result<(), &'static str> {
    if fail {
        Err(QUESTION_ERROR)
    } else {
        Ok(())
    }
}

#[datastore_span(name = "test_operation", system = "postgresql", fields(limit = limit))]
async fn operation(
    limit: usize,
    direct_error: bool,
    question_error: bool,
) -> Result<usize, &'static str> {
    if direct_error {
        return Err(DIRECT_ERROR);
    }
    question_path(question_error)?;
    Ok(limit)
}

#[datastore_span(name = "slow_test_operation", system = "postgresql")]
async fn slow_operation(delay: std::time::Duration) -> Result<(), &'static str> {
    tokio::time::sleep(delay).await;
    Err(DIRECT_ERROR)
}

#[derive(Default)]
struct EventFields(BTreeMap<String, String>);

impl Visit for EventFields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
}

#[derive(Clone, Default)]
struct EventCapture(Arc<Mutex<Vec<EventFields>>>);

impl<S> Layer<S> for EventCapture
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = EventFields::default();
        event.record(&mut fields);
        self.0.lock().expect("capture lock").push(fields);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn exports_policy_fields_without_error_or_argument_data() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _metrics_guard = metrics::set_default_local_recorder(&recorder);
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("datastore-macro-test")));
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);

    assert_eq!(operation(7, false, false).await, Ok(7));
    assert_eq!(operation(8, true, false).await, Err(DIRECT_ERROR));
    assert_eq!(operation(9, false, true).await, Err(QUESTION_ERROR));

    let operation_samples = snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, ..)| key.key().name() == "buzz_db_operation_duration_seconds")
        .map(|(key, _, _, value)| {
            let DebugValue::Histogram(samples) = value else {
                panic!("operation duration must be a histogram");
            };
            let labels = key
                .key()
                .labels()
                .map(|label| (label.key().to_owned(), label.value().to_owned()))
                .collect::<std::collections::BTreeMap<_, _>>();
            (labels, samples)
        })
        .collect::<Vec<_>>();
    assert_eq!(operation_samples.len(), 2);
    for (labels, samples) in operation_samples {
        assert_eq!(
            labels.get("operation").map(String::as_str),
            Some("test_operation")
        );
        assert!(matches!(
            labels.get("outcome").map(String::as_str),
            Some("success" | "error")
        ));
        assert!(!samples.is_empty());
        assert!(samples.iter().all(|sample| sample.into_inner() >= 0.0));
    }

    provider.force_flush().expect("spans flush");
    let spans = exporter.get_finished_spans().expect("exported spans");
    assert_eq!(spans.len(), 3);

    for (span, (expected_limit, expected_status)) in spans.iter().zip([
        (7_i64, Status::Unset),
        (8_i64, Status::error("")),
        (9_i64, Status::error("")),
    ]) {
        assert_eq!(span.name, "test_operation");
        assert_eq!(span.span_kind, SpanKind::Client);
        assert_eq!(span.status, expected_status);

        let attributes = span
            .attributes
            .iter()
            .map(|attribute| (attribute.key.as_str(), attribute.value.to_string()))
            .collect::<Vec<_>>();
        assert!(attributes.contains(&("target", "buzz_datastore".to_owned())));
        assert!(attributes.contains(&("db.system.name", "postgresql".to_owned())));
        assert!(attributes.contains(&("limit", expected_limit.to_string())));
        assert!(!attributes
            .iter()
            .any(|(key, _)| { matches!(*key, "direct_error" | "question_error") }));

        let exported = format!("{span:?}");
        assert!(!exported.contains(DIRECT_ERROR));
        assert!(!exported.contains(QUESTION_ERROR));
        assert!(span.events.iter().all(|event| {
            !format!("{event:?}").contains(DIRECT_ERROR)
                && !format!("{event:?}").contains(QUESTION_ERROR)
        }));
        if let Status::Error { description } = &span.status {
            assert!(description.is_empty());
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn slow_operation_logging_is_guarded_sampled_and_redacted() {
    let capture = EventCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);

    assert_eq!(
        slow_operation(std::time::Duration::from_millis(1)).await,
        Err(DIRECT_ERROR)
    );
    assert_eq!(
        slow_operation(std::time::Duration::from_millis(510)).await,
        Err(DIRECT_ERROR)
    );
    assert_eq!(
        slow_operation(std::time::Duration::from_millis(510)).await,
        Err(DIRECT_ERROR)
    );

    let events = capture.0.lock().expect("capture lock");
    let slow = events
        .iter()
        .filter(|event| {
            event
                .0
                .get("message")
                .is_some_and(|message| message.contains("slow datastore operation"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        slow.len(),
        1,
        "first slow call is logged, next 99 are sampled out"
    );
    let fields = &slow[0].0;
    assert_eq!(
        fields.get("operation").map(String::as_str),
        Some("slow_test_operation")
    );
    assert_eq!(fields.get("outcome").map(String::as_str), Some("error"));
    assert!(fields
        .get("elapsed_ms")
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|elapsed| elapsed >= 500));
    assert_eq!(
        fields.len(),
        4,
        "only message and fixed safe fields are logged"
    );
    assert!(!format!("{fields:?}").contains(DIRECT_ERROR));
}
