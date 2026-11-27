//! CEL programs for webhook subscriptions.
//!
//! A subscription routes an integration's webhook deliveries into a channel.
//! It holds a filter, a CEL expression that decides whether a delivery reaches
//! the channel, and an optional message template, a CEL expression that renders the
//! text the channel receives. Both read two variables:
//!
//! - `event`: `event.provider` and `event.type`;
//! - `payload`: the provider's JSON body.
//!
//! The management API checks a program when it is configured, so a typo is refused
//! with the admin's request instead of failing every later delivery. CEL here is
//! dynamically typed, so the check cannot prove the result type: it refuses a
//! program whose result is a literal of the wrong type, and evaluation refuses
//! any other wrong result.
//!
//! Fan-out then evaluates each program against a [`Delivery`] within a
//! [`Budget`].

mod eval;

pub use eval::{Budget, Delivery, EvalError};

use cel::common::ast::{EntryExpr, Expr, LiteralValue, MapExpr, StructExpr};
use cel::{Env, IdedExpr};

/// The largest filter or message template source accepted, in bytes.
pub const MAX_EXPRESSION_BYTES: usize = 4 * 1024;

/// The deepest bracket and call nesting the parser accepts. Its default is
/// 96, and each level costs it tens of kilobytes of stack.
const MAX_NESTING: u16 = 32;

/// The deepest syntax tree a program may have. Walking, evaluating and
/// dropping a tree all recurse once per level, so this bounds the stack every
/// later step needs. A field chain such as `payload.a.a.a…` is not limited by
/// [`MAX_NESTING`].
const MAX_DEPTH: usize = 64;

/// The stack of the thread that parses a program. Parsing a source of
/// [`MAX_EXPRESSION_BYTES`] can need several megabytes, more than an async
/// worker has. The memory is reserved, and only touched as it is used.
const PARSE_STACK_BYTES: usize = 16 * 1024 * 1024;

/// The variables a webhook program can read.
pub const VARIABLES: [&str; 2] = ["event", "payload"];

/// Type names, which CEL resolves like variables (`type(x) == int`).
const TYPE_NAMES: [&str; 10] = [
    "bool",
    "bytes",
    "double",
    "int",
    "list",
    "map",
    "null_type",
    "string",
    "type",
    "uint",
];

/// Why a webhook program was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompileError {
    /// The source is empty or only whitespace.
    #[error("expression is empty")]
    Empty,
    /// The source is over [`MAX_EXPRESSION_BYTES`].
    #[error("expression exceeds {MAX_EXPRESSION_BYTES} bytes")]
    TooLarge,
    /// The source is not valid CEL, or nests too deeply.
    #[error("{0}")]
    Syntax(String),
    /// The program reads a variable other than [`VARIABLES`].
    #[error("unknown variable `{0}`: use `event` or `payload`")]
    UnknownVariable(String),
    /// The program calls a function the CEL standard library lacks.
    #[error("unknown function `{0}`")]
    UnknownFunction(String),
    /// The program's result is a literal of the wrong type.
    #[error("expression must return {0}")]
    WrongResult(&'static str),
    /// The thread that parses programs could not start. Not the program's
    /// fault.
    #[error("could not start the expression parser: {0}")]
    Unavailable(String),
}

/// What a program must return.
#[derive(Clone, Copy)]
enum Returns {
    Bool,
    String,
}

/// A checked filter: a CEL expression that returns a boolean.
#[derive(Debug, Clone)]
pub struct Filter {
    expression: IdedExpr,
}

/// A checked message template: a CEL expression that returns a string.
#[derive(Debug, Clone)]
pub struct MessageTemplate {
    expression: IdedExpr,
}

impl Filter {
    /// The parsed expression.
    pub fn expression(&self) -> &IdedExpr {
        &self.expression
    }
}

impl MessageTemplate {
    /// The parsed expression.
    pub fn expression(&self) -> &IdedExpr {
        &self.expression
    }
}

/// Check `source` as a filter.
pub fn compile_filter(source: &str) -> Result<Filter, CompileError> {
    compile(source, Returns::Bool).map(|expression| Filter { expression })
}

/// Check `source` as a message template.
pub fn compile_message_template(source: &str) -> Result<MessageTemplate, CompileError> {
    compile(source, Returns::String).map(|expression| MessageTemplate { expression })
}

fn compile(source: &str, returns: Returns) -> Result<IdedExpr, CompileError> {
    if source.trim().is_empty() {
        return Err(CompileError::Empty);
    }
    if source.len() > MAX_EXPRESSION_BYTES {
        return Err(CompileError::TooLarge);
    }
    // A tree too deep for the caller's stack is built, refused and dropped on
    // the parsing thread; only a tree within MAX_DEPTH leaves it.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("webhook-filter-parse".into())
            .stack_size(PARSE_STACK_BYTES)
            .spawn_scoped(scope, || parse_and_check(source, returns))
            .map_err(|error| CompileError::Unavailable(error.to_string()))?
            .join()
            .unwrap_or_else(|_| {
                Err(CompileError::Syntax(
                    "expression could not be parsed".into(),
                ))
            })
    })
}

fn parse_and_check(source: &str, returns: Returns) -> Result<IdedExpr, CompileError> {
    let env = Env::stdlib();
    let expression = env
        .parser()
        .max_recursion_depth(MAX_NESTING)
        .parse(source)
        .map_err(|errors| CompileError::Syntax(errors.to_string()))?;
    if depth(&expression) > MAX_DEPTH {
        return Err(CompileError::Syntax(format!(
            "expression nests more than {MAX_DEPTH} levels"
        )));
    }

    let references = expression.references();
    let mut variables = references.variables();
    variables.sort_unstable();
    if let Some(unknown) = variables
        .into_iter()
        .find(|name| !VARIABLES.contains(name) && !TYPE_NAMES.contains(name))
    {
        return Err(CompileError::UnknownVariable(unknown.to_owned()));
    }
    let mut functions = references.functions();
    functions.sort_unstable();
    if let Some(unknown) = functions
        .into_iter()
        .find(|name| !is_operator(name) && env.function(name).is_none())
    {
        return Err(CompileError::UnknownFunction(unknown.to_owned()));
    }

    check_result(&expression.expr, returns)?;
    Ok(expression)
}

/// The depth of `root`'s tree, found without recursion.
fn depth(root: &IdedExpr) -> usize {
    let mut deepest = 0;
    let mut pending = vec![(root, 1)];
    while let Some((node, level)) = pending.pop() {
        deepest = deepest.max(level);
        pending.extend(
            children(&node.expr)
                .into_iter()
                .map(|child| (child, level + 1)),
        );
    }
    deepest
}

fn children(expr: &Expr) -> Vec<&IdedExpr> {
    match expr {
        Expr::Call(call) => call
            .target
            .as_deref()
            .into_iter()
            .chain(&call.args)
            .collect(),
        Expr::Comprehension(comprehension) => vec![
            &comprehension.iter_range,
            &comprehension.accu_init,
            &comprehension.loop_cond,
            &comprehension.loop_step,
            &comprehension.result,
        ],
        Expr::List(list) => list.elements.iter().collect(),
        Expr::Map(MapExpr { entries, .. }) | Expr::Struct(StructExpr { entries, .. }) => entries
            .iter()
            .flat_map(|entry| match &entry.expr {
                EntryExpr::MapEntry(entry) => vec![&entry.key, &entry.value],
                EntryExpr::StructField(field) => vec![&field.value],
            })
            .collect(),
        Expr::Select(select) => vec![select.operand.as_ref()],
        Expr::Unspecified | Expr::Ident(_) | Expr::Literal(_) => Vec::new(),
    }
}

/// Operators are calls too, named with symbols (`_&&_`, `!_`, `@in`) that a
/// function name cannot contain. The interpreter implements them itself.
fn is_operator(name: &str) -> bool {
    !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Refuse a result that is a literal of the wrong type. Anything computed is
/// checked when it is evaluated.
fn check_result(expr: &Expr, returns: Returns) -> Result<(), CompileError> {
    let wrong = match (returns, expr) {
        (Returns::Bool, Expr::Literal(LiteralValue::Boolean(_))) => false,
        (Returns::Bool, Expr::Literal(_) | Expr::List(_) | Expr::Map(_) | Expr::Struct(_)) => true,
        (Returns::String, Expr::Literal(LiteralValue::String(_))) => false,
        (Returns::String, Expr::Literal(_) | Expr::List(_) | Expr::Map(_) | Expr::Struct(_)) => {
            true
        }
        _ => false,
    };
    if wrong {
        return Err(CompileError::WrongResult(match returns {
            Returns::Bool => "a boolean",
            Returns::String => "a string",
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_programs_over_event_and_payload() {
        let clauses = (1..=24)
            .map(|i| format!("payload.f{i} == {i}"))
            .collect::<Vec<_>>()
            .join(" && ");
        for source in [
            clauses.as_str(),
            r#"event.type == "pull_request" && has(payload.action) && payload.action == "opened""#,
            r#"event.provider == "github""#,
            r#"payload.labels.exists(l, l.name.startsWith("sev"))"#,
            r#"size(payload.title) > 3 && payload.title.matches("^\\[P[0-2]\\]")"#,
            r#"type(payload.count) == int && payload.count in [1, 2]"#,
            "true",
        ] {
            compile_filter(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
        for source in [
            r#""PR #" + string(payload.number) + ": " + payload.title"#,
            "payload.title",
            r#"has(payload.issue) ? payload.issue.title : "no issue""#,
            r#""fixed""#,
        ] {
            compile_message_template(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        }
    }

    #[test]
    fn refuses_bad_programs() {
        // Each of these needs more stack to parse than a test thread has.
        let nested = format!("{}true{}", "(".repeat(2000), ")".repeat(2000));
        let calls = format!("{}payload{} == 1", "size(".repeat(680), ")".repeat(680));
        let fields = format!("payload{} == 1", ".a".repeat(2040));
        let methods = format!("payload{} == 1", ".f()".repeat(1000));
        let too_large = format!("payload.a == \"{}\"", "x".repeat(MAX_EXPRESSION_BYTES));
        for (source, expected) in [
            ("", CompileError::Empty),
            ("  \n", CompileError::Empty),
            (too_large.as_str(), CompileError::TooLarge),
            (
                "payload.a ==",
                CompileError::Syntax(String::new()), // message checked below
            ),
            (nested.as_str(), CompileError::Syntax(String::new())),
            (calls.as_str(), CompileError::Syntax(String::new())),
            (fields.as_str(), CompileError::Syntax(String::new())),
            (methods.as_str(), CompileError::Syntax(String::new())),
            (
                "request.action == \"opened\"",
                CompileError::UnknownVariable("request".into()),
            ),
            (
                "lower(payload.a) == \"x\"",
                CompileError::UnknownFunction("lower".into()),
            ),
            ("1", CompileError::WrongResult("a boolean")),
            ("\"yes\"", CompileError::WrongResult("a boolean")),
            ("[true]", CompileError::WrongResult("a boolean")),
            ("{\"a\": true}", CompileError::WrongResult("a boolean")),
        ] {
            let error = compile_filter(source).expect_err(source);
            match expected {
                CompileError::Syntax(_) => {
                    assert!(
                        matches!(error, CompileError::Syntax(ref m) if !m.is_empty()),
                        "{source}: {error}"
                    )
                }
                expected => assert_eq!(error, expected, "{source}"),
            }
        }
        for source in ["true", "[\"a\"]", "1", r#"{"a": "b"}"#] {
            assert_eq!(
                compile_message_template(source).expect_err(source),
                CompileError::WrongResult("a string"),
                "{source}"
            );
        }
    }
}
