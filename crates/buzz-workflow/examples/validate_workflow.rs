//! Validate a workflow YAML file without publishing it.
//!
//! `parse_yaml` covers schema + definition validation (including the
//! `request_approval.from` rule), but a `message_posted` filter is an evalexpr
//! expression that is only evaluated at trigger time — a malformed one fails
//! silently in production, never firing. This also compiles the filter against
//! a representative context so that class of error surfaces before publishing.
//!
//!   cargo run -p buzz-workflow --example validate_workflow -- <file.yaml> [author_hex] [text] [reply_author_hex] [reply_text]
//!   cargo run -p buzz-workflow --example validate_workflow -- --emit-json <file.yaml> [author_hex] [text] [reply_author_hex] [reply_text]

use buzz_workflow::executor::TriggerContext;
use buzz_workflow::schema::TriggerDef;

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let emit_json = args.first().is_some_and(|arg| arg == "--emit-json");
    if emit_json {
        args.remove(0);
    }
    let mut args = args.into_iter();
    let path = args
        .next()
        .expect("usage: validate_workflow [--emit-json] <file.yaml> [author] [text] [reply_author] [reply_text]");
    let author = args.next().unwrap_or_default();
    let text = args.next().unwrap_or_default();
    let reply_to_author = args.next().unwrap_or_default();
    let reply_to_text = args.next().unwrap_or_default();

    let yaml = std::fs::read_to_string(&path).expect("read yaml");

    let (def, json) = match buzz_workflow::WorkflowEngine::parse_yaml(&yaml) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("INVALID {path}\n  {e}");
            std::process::exit(1);
        }
    };

    if !emit_json {
        println!("valid   {path}");
        println!("  name  {}", def.name);
        println!("  steps {}", def.steps.len());
    }

    if let TriggerDef::MessagePosted { filter: Some(f) }
    | TriggerDef::DiffPosted { filter: Some(f) } = &def.trigger
    {
        let ctx = TriggerContext {
            text: text.clone(),
            author: author.clone(),
            channel_id: "00000000-0000-0000-0000-000000000000".to_owned(),
            timestamp: "1700000000".to_owned(),
            emoji: String::new(),
            message_id: "0".repeat(64),
            reply_to_text,
            reply_to_author,
            reply_to_message_id: String::new(),
            webhook_fields: Default::default(),
        };
        match buzz_workflow::executor::evaluate_condition(f, &ctx, &Default::default()).await {
            Ok(fires) => {
                if !emit_json {
                    println!("  filter compiles; against the sample context it fires: {fires}");
                }
            }
            Err(e) => {
                eprintln!("  FILTER ERROR: {e}");
                eprintln!(
                    "  A filter that cannot evaluate never fires — this would be a silent no-op."
                );
                std::process::exit(2);
            }
        }
    }

    if emit_json {
        println!("{json}");
    }
}
