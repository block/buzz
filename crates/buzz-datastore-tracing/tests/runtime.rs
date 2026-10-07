use buzz_datastore_tracing::datastore_span;
use opentelemetry::trace::{SpanKind, Status, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
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

#[tokio::test(flavor = "current_thread")]
async fn exports_policy_fields_without_error_or_argument_data() {
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
