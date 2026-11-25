//! Evaluating checked programs against a delivery, within a [`Budget`].
//!
//! A program's cost is bounded three ways: its syntax tree by compile
//! ([`MAX_EXPRESSION_BYTES`](crate::MAX_EXPRESSION_BYTES) and the depth
//! limit), its loops by the budget's iteration count and time, which the
//! interpreter checks on every comprehension iteration, and its result by the
//! budget's output size. Without loops, a program's work is bounded by its
//! size and the payload's.
//!
//! The time limit also bounds memory: a program that copies the payload on
//! each iteration allocates until the deadline stops it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use cel::objects::{Key, Map};
use cel::{Context, Deadline, Env, ExecutionError, Interrupt, Value};

use crate::{Filter, Transform, PARSE_STACK_BYTES};

/// Why evaluating a program failed. Each is the program's fault, or its
/// input's, except [`EvalError::Unavailable`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvalError {
    /// CEL raised an error, such as a missing field or no matching overload.
    #[error("{0}")]
    Failed(String),
    /// The program ran more comprehension iterations than the budget allows.
    #[error("expression exceeded {0} loop iterations")]
    TooManyIterations(u64),
    /// The program ran longer than the budget allows.
    #[error("expression exceeded {0:?}")]
    TimedOut(Duration),
    /// The program returned a value of the wrong type.
    #[error("expression must return {0}")]
    WrongResult(&'static str),
    /// The transform's result serializes to more than the budget allows.
    #[error("transform result exceeds {0} bytes")]
    TooLarge(usize),
    /// The relay could not start the thread that evaluates programs. Not the
    /// program's fault.
    #[error("could not start the expression evaluator: {0}")]
    Unavailable(String),
}

/// The limits on one program's evaluation.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    /// Most comprehension iterations, nested ones included.
    pub max_iterations: u64,
    /// Longest evaluation.
    pub timeout: Duration,
    /// Largest transform result, serialized as JSON.
    pub max_output_bytes: usize,
}

/// A delivery as programs read it, converted once and shared by every
/// subscription it is evaluated for.
#[derive(Debug, Clone)]
pub struct Delivery {
    event: Value,
    payload: Value,
}

impl Delivery {
    /// The delivery's provider, event type, and the provider's JSON body.
    ///
    /// A JSON number becomes an `int` when it is an integer that fits one,
    /// otherwise a `uint` when it fits that, and a `double` otherwise, so
    /// `payload.number == 42` and `type(payload.number) == int` hold for 42.
    pub fn new(provider: &str, event_type: &str, payload: &serde_json::Value) -> Self {
        let event = HashMap::from([
            ("provider", Value::from(provider)),
            ("type", Value::from(event_type)),
        ]);
        Self {
            event: event.into(),
            payload: to_value(payload),
        }
    }
}

impl Filter {
    /// Whether `delivery` matches this filter.
    pub fn matches(&self, delivery: &Delivery, budget: &Budget) -> Result<bool, EvalError> {
        match evaluate(self.expression(), delivery, budget)? {
            Value::Bool(matched) => Ok(matched),
            _ => Err(EvalError::WrongResult("a boolean")),
        }
    }
}

impl Transform {
    /// What this transform makes of `delivery`: a JSON object.
    pub fn apply(
        &self,
        delivery: &Delivery,
        budget: &Budget,
    ) -> Result<serde_json::Map<String, serde_json::Value>, EvalError> {
        let value = evaluate(self.expression(), delivery, budget)?;
        if !matches!(value, Value::Map(_)) {
            return Err(EvalError::WrongResult("a map"));
        }
        let Ok(serde_json::Value::Object(object)) = value.json() else {
            return Err(EvalError::WrongResult("a map of JSON values"));
        };
        // Count the serialized size without keeping the bytes.
        let mut counter = Counter {
            written: 0,
            limit: budget.max_output_bytes,
        };
        serde_json::to_writer(&mut counter, &object)
            .map_err(|_| EvalError::TooLarge(budget.max_output_bytes))?;
        Ok(object)
    }
}

/// The environment every program runs in: the standard library. Its own
/// iteration limit is unset; [`evaluate`]'s interrupt counts iterations
/// against each budget instead.
static ENV: LazyLock<Arc<Env>> = LazyLock::new(|| Arc::new(Env::stdlib()));

fn evaluate(
    expression: &cel::IdedExpr,
    delivery: &Delivery,
    budget: &Budget,
) -> Result<Value, EvalError> {
    // Evaluation recurses once per tree level, as parsing does, so it runs
    // where the stack fits the deepest tree compile admits.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("webhook-filter-eval".into())
            .stack_size(PARSE_STACK_BYTES)
            .spawn_scoped(scope, || {
                // The interpreter polls this once per comprehension
                // iteration, nested ones included.
                let deadline = Deadline::after(budget.timeout);
                let iterations = AtomicU64::new(0);
                let out_of_iterations = AtomicBool::new(false);
                let interrupt = || {
                    if iterations.fetch_add(1, Ordering::Relaxed) >= budget.max_iterations {
                        out_of_iterations.store(true, Ordering::Relaxed);
                        return true;
                    }
                    deadline.is_interrupted()
                };
                let mut context = Context::with_env(Arc::clone(&ENV));
                context.add_variable_from_value("event", delivery.event.clone());
                context.add_variable_from_value("payload", delivery.payload.clone());
                context.set_interrupt(&interrupt);
                context.resolve(expression).map_err(|error| match error {
                    ExecutionError::Interrupted if out_of_iterations.load(Ordering::Relaxed) => {
                        EvalError::TooManyIterations(budget.max_iterations)
                    }
                    ExecutionError::Interrupted => EvalError::TimedOut(budget.timeout),
                    other => EvalError::Failed(other.to_string()),
                })
            })
            .map_err(|error| EvalError::Unavailable(error.to_string()))?
            .join()
            .unwrap_or_else(|_| {
                Err(EvalError::Failed(
                    "expression could not be evaluated".into(),
                ))
            })
    })
}

fn to_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(Value::Int)
            .or_else(|| n.as_u64().map(Value::UInt))
            .unwrap_or_else(|| Value::Float(n.as_f64().unwrap_or(f64::NAN))),
        serde_json::Value::String(s) => Value::from(s.as_str()),
        serde_json::Value::Array(items) => {
            Value::List(Arc::new(items.iter().map(to_value).collect()))
        }
        serde_json::Value::Object(fields) => Value::Map(Map {
            map: Arc::new(
                fields
                    .iter()
                    .map(|(k, v)| (Key::from(k.as_str()), to_value(v)))
                    .collect(),
            ),
        }),
    }
}

/// A writer that only counts, and fails past `limit` bytes.
struct Counter {
    written: usize,
    limit: usize,
}

impl std::io::Write for Counter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.written += buf.len();
        if self.written > self.limit {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile_filter, compile_transform};
    use serde_json::json;

    const BUDGET: Budget = Budget {
        max_iterations: 10_000,
        timeout: Duration::from_secs(5),
        max_output_bytes: 1024,
    };

    fn delivery() -> Delivery {
        let items: Vec<u64> = (0..200).collect();
        Delivery::new(
            "github",
            "pull_request.opened",
            &json!({
                "action": "opened", "number": 42, "big": u64::MAX, "ratio": 1.5,
                "title": "Fix the build", "labels": [{"name": "sev1"}], "items": items,
            }),
        )
    }

    #[test]
    fn filters_match_deliveries() {
        let delivery = delivery();
        for (source, expected) in [
            (
                r#"event.provider == "github" && event.type == "pull_request.opened""#,
                Ok(true),
            ),
            (
                r#"has(payload.action) && payload.action == "opened""#,
                Ok(true),
            ),
            (r#"payload.action == "closed""#, Ok(false)),
            ("has(payload.missing)", Ok(false)),
            (
                "payload.number == 42 && type(payload.number) == int",
                Ok(true),
            ),
            (
                "type(payload.big) == uint && type(payload.ratio) == double",
                Ok(true),
            ),
            (
                r#"payload.labels.exists(l, l.name.startsWith("sev"))"#,
                Ok(true),
            ),
            ("payload.items.exists(i, i == 199)", Ok(true)),
            ("payload.action", Err(EvalError::WrongResult("a boolean"))),
            (
                "payload.missing == 1",
                Err(EvalError::Failed(String::new())),
            ),
            // 200 × 200 iterations, and `||` cannot absorb the refusal.
            (
                "payload.items.all(a, payload.items.all(b, true)) || true",
                Err(EvalError::TooManyIterations(10_000)),
            ),
        ] {
            let result = compile_filter(source).unwrap().matches(&delivery, &BUDGET);
            match expected {
                Err(EvalError::Failed(_)) => {
                    assert!(
                        matches!(result, Err(EvalError::Failed(ref m)) if !m.is_empty()),
                        "{source}: {result:?}"
                    )
                }
                expected => assert_eq!(result, expected, "{source}"),
            }
        }

        let slow = Budget {
            max_iterations: u64::MAX,
            timeout: Duration::ZERO,
            ..BUDGET
        };
        assert_eq!(
            compile_filter("payload.items.all(i, true)")
                .unwrap()
                .matches(&delivery, &slow),
            Err(EvalError::TimedOut(Duration::ZERO))
        );
    }

    #[test]
    fn transforms_shape_deliveries() {
        let delivery = delivery();
        let transform = |source: &str, budget: &Budget| {
            compile_transform(source).unwrap().apply(&delivery, budget)
        };
        assert_eq!(
            transform(
                r#"{"title": payload.title, "n": payload.number, "names": payload.labels.map(l, l.name)}"#,
                &BUDGET
            ),
            Ok(
                json!({"title": "Fix the build", "n": 42, "names": ["sev1"]})
                    .as_object()
                    .cloned()
                    .unwrap()
            )
        );
        assert_eq!(
            transform("payload.title", &BUDGET),
            Err(EvalError::WrongResult("a map"))
        );
        let tight = Budget {
            max_output_bytes: 24,
            ..BUDGET
        };
        assert!(transform(r#"{"t": payload.title}"#, &tight).is_ok());
        assert_eq!(
            transform(r#"{"t": payload.title, "a": payload.action}"#, &tight),
            Err(EvalError::TooLarge(24))
        );
    }
}
