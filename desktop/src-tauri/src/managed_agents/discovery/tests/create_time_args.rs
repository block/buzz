//! `create_time_agent_args`: which authority decides a new record's
//! `agent_args`. A local create is normalized against the LOCAL runtime
//! catalog; a provider create is pinned verbatim from the REMOTE host's.
//!
//! Every test below pairs the two backends over the SAME input, because the
//! bug this guards against is invisible otherwise: a host binary whose
//! basename collides with a local runtime is normalized without complaint.

use crate::managed_agents::{discovery::create_time_agent_args, BackendKind};

fn provider() -> BackendKind {
    BackendKind::Provider {
        id: "ssh".to_string(),
        config: serde_json::json!({}),
    }
}

#[test]
fn create_time_args_normalize_against_the_local_catalog_for_a_local_create() {
    // A local record re-resolves its args from the definition on every spawn,
    // so the local catalog is the right authority here: `goose` gains its
    // default `acp`, and a buzz-agent create's stray `acp` is dropped.
    assert_eq!(
        create_time_agent_args(&BackendKind::Local, "goose", &[]),
        vec!["acp".to_string()]
    );
    assert_eq!(
        create_time_agent_args(&BackendKind::Local, "buzz-agent", &["acp".to_string()]),
        Vec::<String>::new()
    );
}

#[test]
fn create_time_args_pin_a_provider_create_verbatim() {
    // The discriminator: a REMOTE binary whose basename collides with a local
    // runtime. `/opt/hosttools/bin/buzz-agent` normalizes to the local
    // `buzz-agent` identity, whose default args are empty, so the local path
    // deletes a lone `acp` — even though the HOST's own catalog is what
    // reported that argument. The deploy payload reads `record.agent_args`
    // directly, with no second resolution, so the host binary would launch
    // bare and the create's harness choice would be silently wrong.
    let pinned = ["acp".to_string()];

    assert_eq!(
        create_time_agent_args(&provider(), "/opt/hosttools/bin/buzz-agent", &pinned),
        pinned.to_vec()
    );
    assert_eq!(
        create_time_agent_args(
            &BackendKind::Local,
            "/opt/hosttools/bin/buzz-agent",
            &pinned
        ),
        Vec::<String>::new(),
        "the local path is what would have destroyed the pin"
    );
}

#[test]
fn create_time_args_never_invent_args_for_a_provider_create() {
    // The mirror of the case above: a host harness that takes no arguments
    // must not acquire the local `goose` default. Only the host's catalog
    // decides what its own binary is launched with.
    assert_eq!(
        create_time_agent_args(&provider(), "/opt/hosttools/bin/goose", &[]),
        Vec::<String>::new()
    );
    assert_eq!(
        create_time_agent_args(&BackendKind::Local, "/opt/hosttools/bin/goose", &[]),
        vec!["acp".to_string()],
        "the local path is what would have invented the argument"
    );
    // Blanks are still dropped — an empty string is a serialization artifact,
    // never a real argument, on either backend.
    assert_eq!(
        create_time_agent_args(&provider(), "/opt/hosttools/bin/goose", &[" ".to_string()]),
        Vec::<String>::new()
    );
}
