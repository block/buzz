use serde_json::json;

use super::*;

const PUBKEY: &str = "7e7e9c42a91bfef19fa929e5fda1b72e0ebc1a4c1141673e2794234d86addf4e";
const CHANNEL: &str = "9b353519-0d6a-4c55-9a8e-5d3c8b1f2a10";

fn live() -> AgentDocument {
    serde_json::from_value(json!({
        "version": 1,
        "agent": {"pubkey": PUBKEY, "community": "wss://relay.example.org", "owner": PUBKEY},
        "type": "claude-code",
        "profile": {"name": "Triage Bot", "about": "First responder for pages.",
                    "picture": "https://media.example.org/triage.png"},
        "instructions": "You are Triage Bot.\n\n  Keep replies short.\n",
        "settings": {"model": "model-large", "effort": "high", "scope": "community"},
        "attention": {"objects": {
            "interest/pages": {"type": "interest", "title": "Pages",
                               "instructions": "Confirm the page and open a thread."},
            "interest/digest": {"type": "interest",
                                "instructions": "Post a summary of open pages."},
            "interest/notes": {"type": "interest", "summary": "Things to remember",
                               "instructions": "Keep notes when asked."},
            "watch/new-pages": {"type": "event", "interest_id": "pages", "enabled": true,
                                "channels": [CHANNEL], "kinds": [9],
                                "classifier": {"questions": {
                                    "is_page": {"question": "Is this a new page?"},
                                    "urgent": {"question": "Is it urgent?", "true": "Someone is waiting",
                                               "threshold": 0.6}}}},
            "watch/morning": {"type": "timer", "interest_id": "pages", "enabled": true,
                              "prompt": "Look for pages with no owner.", "interval_secs": 86400,
                              "max_occurrences": null, "expires_at": null},
            "watch/weekly": {"type": "timer", "interest_id": "digest", "enabled": false,
                             "prompt": "Write the digest.", "interval_secs": 5400,
                             "max_occurrences": 3, "expires_at": null}
        }},
        "channels": [{"channel": CHANNEL, "role": "member"}]
    }))
    .unwrap()
}

fn rendered() -> String {
    render(&live(), "triage-bot").unwrap()
}

#[test]
fn export_writes_the_canonical_file() {
    let text = rendered();
    let npub = codec::npub(PUBKEY).unwrap();
    let expected = format!(
        r#"---
kind: Agent
version: 3
name: triage-bot
display_name: Triage Bot
description: First responder for pages.
enabled: true
identity:
  buzz: {npub}
avatar: https://media.example.org/triage.png
model: model-large
effort: high
buzz:
  community: wss://relay.example.org
  runtime: claude-code
  settings:
    scope: community
  channels:
  - channel: {CHANNEL}
    role: member
  interests:
  - id: notes
    summary: Things to remember
    instructions: Keep notes when asked.
  - id: pages
    title: Pages
wake:
  timers:
  - id: morning
    enabled: true
    every: 1d
    max: unbounded
    interest: pages
    prompt: Look for pages with no owner.
  - id: weekly
    enabled: false
    every: 90m
    max: 3
    interest: digest
    prompt: Write the digest.
  events:
  - id: new-pages
    enabled: true
    source: buzz
    interest: pages
    channels:
    - {CHANNEL}
    kinds:
    - 9
    classifier:
      is_page: Is this a new page?
      urgent:
        question: Is it urgent?
        'true': Someone is waiting
        threshold: 0.6
---

You are Triage Bot.

  Keep replies short.

## When morning, new-pages

Confirm the page and open a thread.

## When weekly

Post a summary of open pages.
"#
    );
    assert_eq!(text, expected);
}

#[test]
fn a_round_trip_gives_the_same_agent_and_bytes() {
    let text = rendered();
    let parsed = parse(&text).unwrap();
    assert_eq!(parsed.name, "triage-bot");
    let mut expected = live();
    expected.agent.as_mut().unwrap().owner = None;
    expected.instructions = "You are Triage Bot.\n\n  Keep replies short.".into();
    assert_eq!(parsed.document, expected);
    assert_eq!(render(&parsed.document, &parsed.name).unwrap(), text);
}

#[test]
fn comments_reordering_and_spacing_are_no_change() {
    let text = rendered()
        .replace("kind: Agent\n", "# Edited by hand\nkind: Agent\n")
        .replace(
            "model: model-large\neffort: high\n",
            "effort: high\nmodel: model-large # pinned\n",
        )
        .replace("## When weekly\n\n", "## When weekly\n\n\n");
    let desired = parse(&text).unwrap().document;
    let plan = plan(Some(&live()), &desired, "triage-bot").unwrap();
    assert_eq!(plan.action, Action::NoChange);
    assert_eq!(plan.report("triage-bot"), "NO_CHANGE triage-bot\n");
}

#[test]
fn a_dry_run_shows_the_change_as_an_agent_md_diff() {
    let text = rendered()
        .replace("    - 9\n", "    - 9\n    - 1111\n")
        .replace(
            "Post a summary of open pages.",
            "Post a summary of open and closed pages.",
        );
    let desired = parse(&text).unwrap().document;
    let report = plan(Some(&live()), &desired, "triage-bot")
        .unwrap()
        .report("triage-bot");
    let mut lines = report.lines();
    assert_eq!(
        lines.next(),
        Some("UPDATE triage-bot: interest digest, wake.events[new-pages]")
    );
    assert_eq!(lines.next(), Some("--- live/triage-bot/AGENT.md"));
    assert_eq!(lines.next(), Some("+++ desired/triage-bot/AGENT.md"));
    assert!(report.contains("\n+    - 1111\n"));
    assert!(report
        .contains("\n-Post a summary of open pages.\n+Post a summary of open and closed pages.\n"));
}

#[test]
fn a_file_without_identity_creates_an_agent() {
    let npub = codec::npub(PUBKEY).unwrap();
    let text = rendered()
        .replace(&format!("identity:\n  buzz: {npub}\n"), "")
        .replace("  community: wss://relay.example.org\n", "");
    let desired = parse(&text).unwrap().document;
    assert_eq!(desired.agent, None);
    let plan = plan(None, &desired, "triage-bot").unwrap();
    assert_eq!(plan.action, Action::Create);
    assert!(plan
        .report("triage-bot")
        .starts_with("CREATE triage-bot\n--- /dev/null\n+++ desired/triage-bot/AGENT.md\n"));
}

#[test]
fn a_wake_without_interest_feeds_the_interest_named_by_its_section() {
    let text = "---\nkind: Agent\nversion: 2\nname: a\ndescription: d\nbuzz:\n  runtime: goose\nwake:\n  timers:\n  - id: daily\n    every: 24h\n    prompt: Check.\n  events:\n  - id: mentions\n    source: buzz\n    channels: all\n    kinds: [9]\n---\nPersona.\n\n## When daily, mentions\n\nDo the work.\n";
    let doc = parse(text).unwrap().document;
    let objects = &doc.attention.objects;
    assert!(matches!(
        &objects["interest/daily"],
        document::AttentionValue::Interest(i) if i.instructions == "Do the work."
    ));
    for slug in ["watch/daily", "watch/mentions"] {
        let interest = match &objects[slug] {
            document::AttentionValue::Timer(t) => &t.interest_id,
            document::AttentionValue::Event(e) => &e.interest_id,
            _ => unreachable!(),
        };
        assert_eq!(interest, "daily");
    }
    assert!(matches!(
        &objects["watch/daily"],
        document::AttentionValue::Timer(t) if t.max_occurrences == Some(1) && t.interval_secs == 86_400
    ));
}

#[test]
fn import_refuses_what_a_buzz_agent_cannot_run() {
    let base = rendered();
    for (edit, message) in [
        (
            base.replace("kind: Agent\n", "kind: Agent\nskills: [review]\n"),
            "skills isn't supported",
        ),
        (
            base.replace("wake:\n", "wake:\n  session: continue\n"),
            "wake.session isn't supported",
        ),
        (
            base.replace("enabled: true\nidentity", "enabled: false\nidentity"),
            "enabled: false",
        ),
        (
            base.replace("kind: Agent\n", "kind: Agent\ncolour: blue\n"),
            "unknown field `colour`",
        ),
        (
            base.replace("    every: 90m\n", "    every: 90 minutes\n"),
            "every \"90 minutes\"",
        ),
        (
            base.replace("## When weekly", "## When weekly, nope"),
            "nope, which isn't a wake",
        ),
        (
            base.replace("## When weekly", "## When weekly, morning"),
            "two ## When sections",
        ),
    ] {
        let error = parse(&edit).unwrap_err().0;
        assert!(error.contains(message), "{message:?} not in {error:?}");
    }
}

#[test]
fn export_refuses_what_the_file_cannot_express() {
    let mut doc = live();
    doc.attention.objects.insert(
        "watch/feed".into(),
        serde_json::from_value(
            json!({"type": "source", "interest_id": "pages", "enabled": true,
                                      "provider": "rss", "config": {}}),
        )
        .unwrap(),
    );
    assert!(render(&doc, "triage-bot")
        .unwrap_err()
        .0
        .contains("source watch"));

    let mut doc = live();
    let timer = doc.attention.objects.remove("watch/weekly").unwrap();
    doc.attention
        .objects
        .insert("watch/Weekly_Digest".into(), timer);
    assert!(render(&doc, "triage-bot")
        .unwrap_err()
        .0
        .contains("wake id"));

    let mut doc = live();
    doc.instructions = "Intro\n## When asked\nanswer".into();
    assert!(render(&doc, "triage-bot")
        .unwrap_err()
        .0
        .contains("persona prompt"));
}

#[test]
fn when_headings_that_name_no_wake_ids_or_sit_in_code_stay_prompt_text() {
    // As in squareup/agents: a heading without a comma names one wake id, and
    // one inside fenced code is never a section.
    let persona = "You are Triage Bot.\n\n## When the build fails\n\nSay so.\n\n```markdown\n## When morning\n```\n\n~~~~\n## When a, b\n~~~~";
    let mut doc = live();
    doc.instructions = persona.into();
    let text = render(&doc, "triage-bot").unwrap();
    assert_eq!(parse(&text).unwrap().document.instructions, persona);
    let error = parse(&text.replace("## When weekly", "## When weekly,"))
        .unwrap_err()
        .0;
    assert!(error.contains("empty wake id"), "{error}");

    // A fence left open would swallow the sections after it.
    doc.instructions = "Intro\n```\nunclosed".into();
    let error = render(&doc, "triage-bot").unwrap_err().0;
    assert!(error.contains("unclosed code fence"), "{error}");
}

#[test]
fn classifier_answers_can_be_written_without_quotes() {
    let text = rendered().replace("'true': Someone", "true: Someone");
    assert_eq!(
        parse(&text).unwrap().document,
        parse(&rendered()).unwrap().document
    );
}

#[test]
fn display_names_become_file_names() {
    assert_eq!(slug_name("SEV Triage!"), "sev-triage");
    assert_eq!(slug_name("  Ünïcode  "), "n-code");
    assert_eq!(slug_name("***"), "agent");
}
