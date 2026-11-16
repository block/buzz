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

use buzz_db::error::DatastoreSpanError;
use std::borrow::Cow;

const DIRECT_ERROR: &str = "raw-secret-direct-error";
const QUESTION_ERROR: &str = "raw-secret-question-error";
const BOUND_VALUE: &str = "raw-secret-bound-value";

/// A domain error whose text carries a secret, as tenant data would.
#[derive(Debug, PartialEq, thiserror::Error)]
enum TestError {
    #[error("{0}")]
    Direct(&'static str),
    #[error("{0}")]
    Question(&'static str),
}

impl DatastoreSpanError for TestError {
    fn span_error_type(&self) -> &'static str {
        match self {
            Self::Direct(_) => "direct",
            Self::Question(_) => "question",
        }
    }
}

fn question_path(fail: bool) -> Result<(), TestError> {
    if fail {
        Err(TestError::Question(QUESTION_ERROR))
    } else {
        Ok(())
    }
}

#[datastore_span(name = "test_operation", system = "postgresql", fields(limit = limit))]
async fn operation(
    limit: usize,
    direct_error: bool,
    question_error: bool,
) -> Result<usize, TestError> {
    if direct_error {
        return Err(TestError::Direct(DIRECT_ERROR));
    }
    question_path(question_error)?;
    Ok(limit)
}

#[datastore_span(name = "slow_test_operation", system = "postgresql")]
async fn slow_operation(delay: std::time::Duration) -> Result<(), TestError> {
    tokio::time::sleep(delay).await;
    Err(TestError::Direct(DIRECT_ERROR))
}

/// A Postgres error whose message, detail-like text, code and constraint are
/// all under test control.
#[derive(Debug, thiserror::Error)]
#[error("duplicate key: Key (name)=({BOUND_VALUE}) already exists")]
struct ServerError {
    code: &'static str,
    constraint: &'static str,
}

impl sqlx::error::DatabaseError for ServerError {
    fn message(&self) -> &str {
        "duplicate key: Key (name)=(raw-secret-bound-value) already exists"
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.code))
    }

    fn constraint(&self) -> Option<&str> {
        Some(self.constraint)
    }

    fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> sqlx::error::ErrorKind {
        sqlx::error::ErrorKind::UniqueViolation
    }
}

#[datastore_span(name = "driver_operation", system = "postgresql")]
async fn driver_operation(error: buzz_db::DbError) -> Result<(), buzz_db::DbError> {
    Err(error)
}

#[datastore_span(name = "search_operation", system = "postgresql", pool = "search")]
async fn search_operation() -> Result<(), sqlx::Error> {
    Err(sqlx::Error::PoolTimedOut)
}

fn exported_spans<F: std::future::Future<Output = ()>>(
    run: impl FnOnce() -> F,
) -> Vec<opentelemetry_sdk::trace::SpanData> {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("datastore-macro-test")));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    tracing::subscriber::with_default(subscriber, || runtime.block_on(run()));
    provider.force_flush().expect("spans flush");
    exporter.get_finished_spans().expect("exported spans")
}

/// A span's attributes by key. Panics on a repeated key: a field recorded
/// twice exports two attributes, and a backend may show either value.
fn attributes(span: &opentelemetry_sdk::trace::SpanData) -> BTreeMap<String, String> {
    let attributes = span
        .attributes
        .iter()
        .map(|attribute| {
            (
                attribute.key.as_str().to_owned(),
                attribute.value.to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        attributes.len(),
        span.attributes.len(),
        "repeated attribute: {span:?}"
    );
    attributes
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
    assert_eq!(
        operation(8, true, false).await,
        Err(TestError::Direct(DIRECT_ERROR))
    );
    assert_eq!(
        operation(9, false, true).await,
        Err(TestError::Question(QUESTION_ERROR))
    );

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

    for (span, (expected_limit, expected_status, expected_error_type)) in spans.iter().zip([
        (7_i64, Status::Unset, None),
        (8_i64, Status::error(""), Some("direct")),
        (9_i64, Status::error(""), Some("question")),
    ]) {
        assert_eq!(span.name, "test_operation");
        assert_eq!(span.span_kind, SpanKind::Client);
        assert_eq!(span.status, expected_status);

        attributes(span);
        let attributes = span
            .attributes
            .iter()
            .map(|attribute| (attribute.key.as_str(), attribute.value.to_string()))
            .collect::<Vec<_>>();
        assert!(attributes.contains(&("target", "buzz_datastore".to_owned())));
        assert!(attributes.contains(&("db.system.name", "postgresql".to_owned())));
        assert!(attributes.contains(&("limit", expected_limit.to_string())));
        assert!(attributes.contains(&("db.pool.role", "writer".to_owned())));
        assert_eq!(
            attributes
                .iter()
                .find(|(key, _)| *key == "error.type")
                .map(|(_, value)| value.as_str()),
            expected_error_type
        );
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

    for delay in [1, 510, 510] {
        assert_eq!(
            slow_operation(std::time::Duration::from_millis(delay)).await,
            Err(TestError::Direct(DIRECT_ERROR))
        );
    }

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

#[test]
fn driver_errors_export_sqlstate_and_constraint_but_never_text() {
    let spans = exported_spans(|| async {
        let unique = sqlx::Error::database(ServerError {
            code: "23505",
            constraint: "idx_channels_listed_name",
        });
        assert!(driver_operation(unique.into()).await.is_err());
        // A value that is not a SQLSTATE or a plain identifier is dropped.
        let malformed = sqlx::Error::database(ServerError {
            code: BOUND_VALUE,
            constraint: "Key (name)=(raw-secret-bound-value)",
        });
        assert!(driver_operation(malformed.into()).await.is_err());
        let tenant_text = buzz_db::DbError::ChannelNameTaken(BOUND_VALUE.to_owned());
        assert!(driver_operation(tenant_text).await.is_err());
        assert!(search_operation().await.is_err());
    });
    assert_eq!(spans.len(), 4);

    let unique = attributes(&spans[0]);
    assert_eq!(unique["error.type"], "database");
    assert_eq!(unique["db.response.status_code"], "23505");
    assert_eq!(unique["db.error.constraint"], "idx_channels_listed_name");
    assert_eq!(unique["db.pool.role"], "writer");

    let malformed = attributes(&spans[1]);
    assert_eq!(malformed["error.type"], "database");
    assert!(!malformed.contains_key("db.response.status_code"));
    assert!(!malformed.contains_key("db.error.constraint"));

    let tenant_text = attributes(&spans[2]);
    assert_eq!(tenant_text["error.type"], "channel_name_taken");
    assert!(!tenant_text.contains_key("db.response.status_code"));

    let search = attributes(&spans[3]);
    assert_eq!(search["error.type"], "pool_timed_out");
    assert_eq!(search["db.pool.role"], "search");

    for span in &spans {
        assert_eq!(span.status, Status::error(""));
        assert!(!format!("{span:?}").contains(BOUND_VALUE));
        assert!(!format!("{span:?}").contains("Key (name)"));
    }
}
