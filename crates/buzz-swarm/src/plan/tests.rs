use super::*;
use crate::config::SecretRef;

struct Fixture {
    root: tempfile::TempDir,
    owner: Keys,
}

impl Fixture {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().expect("directory"),
            owner: Keys::generate(),
        }
    }

    fn plan(&self, settings: &str, agents: &str, parent: &Env) -> Result<Plan> {
        let mut file = SwarmFile::parse(&format!(
            "owner:\n  nsec: {{env: OWNER}}\ndefaults:\n  harness: /usr/bin/true\n  relays: [https://relay.example]\n{settings}\nagents:\n{agents}"
        ))?;
        file.directory = self.root.path().to_owned();
        let mut values: Vec<_> = parent
            .keys()
            .map(|key| (key.to_owned(), parent.get(key).unwrap().to_owned()))
            .collect();
        values.push(("OWNER".into(), self.owner.secret_key().to_secret_hex()));
        Plan::build(&file, &[], &Env::from_iter(values))
    }
}

#[test]
fn runtime_configuration_passes_through_without_a_second_provider_contract() {
    let fixture = Fixture::new();
    let plan = fixture.plan(
        "  env:\n    BUZZ_ACP_AGENT_COMMAND: custom-agent\n    BUZZ_ACP_MCP_COMMAND: ''\n    BUZZ_AGENT_PROVIDER: future-provider\n    BUZZ_ACP_SYSTEM_PROMPT: '  preserve spaces  '\n    API_CREDENTIAL: {env: CREDENTIAL}",
        "  - name: helper", &Env::from_iter([("CREDENTIAL", "test-value")]),
    ).expect("plan");
    let env = &plan.agents[0].env;
    assert_eq!(env["BUZZ_ACP_AGENT_COMMAND"], "custom-agent");
    assert_eq!(env["BUZZ_ACP_MCP_COMMAND"], "");
    assert_eq!(env["BUZZ_AGENT_PROVIDER"], "future-provider");
    assert_eq!(env["BUZZ_ACP_SYSTEM_PROMPT"], "  preserve spaces  ");
    assert_eq!(env["API_CREDENTIAL"], "test-value");
    assert!(
        fixture
            .plan("", "  - name: helper", &Env::default())
            .is_ok(),
        "runtime, not swarm, validates provider configuration"
    );
}

#[test]
fn inherited_identity_and_acp_policy_are_removed_but_provider_env_is_inherited() {
    let fixture = Fixture::new();
    let plan = fixture
        .plan(
            "",
            "  - name: helper\n  - name: sibling\n    enabled: false\n    nsec: {env: SIBLING}",
            &Env::from_iter([
                ("SIBLING", "other-key"),
                ("BUZZ_ACP_RESPOND_TO", "anyone"),
                ("BUZZ_API_TOKEN", "stale-token"),
                ("BUZZ_AGENT_PROVIDER", "anthropic"),
            ]),
        )
        .expect("plan");
    let agent = &plan.agents[0];
    for key in ["OWNER", "SIBLING", "BUZZ_ACP_RESPOND_TO", "BUZZ_API_TOKEN"] {
        assert!(
            agent.env_remove.iter().any(|removed| removed == key),
            "{key}"
        );
    }
    assert!(!agent
        .env_remove
        .iter()
        .any(|key| key == "BUZZ_AGENT_PROVIDER"));
    assert_eq!(
        agent.env["BUZZ_PRIVATE_KEY"],
        agent.env["NOSTR_PRIVATE_KEY"]
    );
    assert_eq!(agent.env["BUZZ_ACP_AGENT_COMMAND"], "buzz-agent");
    let agent_keys = Keys::parse(&agent.env["BUZZ_PRIVATE_KEY"]).expect("agent key");
    assert_ne!(fixture.owner.public_key(), agent_keys.public_key());
    assert_eq!(
        buzz_sdk::nip_oa::verify_auth_tag(&agent.env["BUZZ_AUTH_TAG"], &agent_keys.public_key())
            .expect("tag"),
        fixture.owner.public_key()
    );
}

#[test]
fn declared_sources_reach_only_their_declarers() {
    let fixture = Fixture::new();
    let plan = fixture
        .plan(
            "  env:\n    SHARED: {env: DEFAULT_SOURCE}\n    ANTHROPIC_API_KEY: {env: ANTHROPIC_API_KEY}",
            "  - name: alpha\n    env:\n      SHARED: alpha\n      MY_KEY: {env: ALPHA_SOURCE}\n  - name: beta\n    env:\n      SHARED: beta",
            &Env::from_iter([
                ("DEFAULT_SOURCE", "overridden-everywhere"),
                ("ALPHA_SOURCE", "alpha-only"),
                ("ANTHROPIC_API_KEY", "shared-provider-key"),
            ]),
        )
        .expect("plan");
    let [alpha, beta] = &plan.agents[..] else {
        panic!("two connections");
    };
    assert_eq!(alpha.env["MY_KEY"], "alpha-only");
    assert!(!beta.env.contains_key("MY_KEY"));
    for agent in [alpha, beta] {
        assert_eq!(agent.env["ANTHROPIC_API_KEY"], "shared-provider-key");
        for source in ["ALPHA_SOURCE", "DEFAULT_SOURCE", "ANTHROPIC_API_KEY"] {
            assert!(agent.env_remove.iter().any(|key| key == source), "{source}");
        }
    }
}

#[test]
fn env_cannot_override_identity_sources_or_use_invalid_names() {
    let fixture = Fixture::new();
    for key in RESERVED.iter().copied().chain(["OWNER", "BAD=KEY", "1BAD"]) {
        let error = fixture
            .plan(
                &format!("  env:\n    '{key}': forbidden"),
                "  - name: helper",
                &Env::default(),
            )
            .err()
            .expect("invalid env rejected");
        assert!(format!("{error:#}").contains(key), "{key}");
    }
}

#[test]
fn one_identity_is_reused_on_every_relay() {
    let fixture = Fixture::new();
    let plan = fixture
        .plan(
            "",
            "  - name: helper\n    relays: [one.example, two.example, three.example]",
            &Env::default(),
        )
        .expect("plan");
    assert_eq!(plan.generated_keys.len(), 1);
    assert_eq!(plan.agents.len(), 3);
    for connection in &plan.agents {
        assert_eq!(
            connection.env["BUZZ_AUTH_TAG"],
            plan.agents[0].env["BUZZ_AUTH_TAG"]
        );
        assert_eq!(connection.pubkey, plan.agents[0].pubkey);
    }
    assert_eq!(
        plan.agents
            .iter()
            .map(|agent| &agent.relay_url)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
}

#[test]
fn duplicate_connections_and_names_are_rejected() {
    let fixture = Fixture::new();
    for agents in [
        "  - name: helper\n    relays: [https://relay.example, wss://relay.example/]",
        "  - name: helper\n  - name: HELPER",
        "  - name: helper\n    relays: [ws://localhost:3000, ws://127.0.0.1:3000]",
    ] {
        assert!(fixture.plan("", agents, &Env::default()).is_err());
    }
}

#[test]
fn file_credentials_resolve_relative_to_the_config_directory() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.path().join("credential"), "test-value\n").expect("credential");
    let plan = fixture
        .plan(
            "  env:\n    API_CREDENTIAL: {file: credential}",
            "  - name: helper",
            &Env::default(),
        )
        .expect("plan");
    assert_eq!(plan.agents[0].env["API_CREDENTIAL"], "test-value");
    assert_eq!(
        plan.agents[0].workdir,
        fixture.root.path().join("workspaces/helper")
    );
    assert_eq!(plan.new_workdirs, [plan.agents[0].workdir.clone()]);
    // Another agent's pending default does not make an explicit workdir exist.
    for agents in [
        "  - name: helper\n  - name: other\n    workdir: workspaces/helper",
        "  - name: other\n    workdir: workspaces/helper\n  - name: helper",
    ] {
        assert!(fixture.plan("", agents, &Env::default()).is_err());
    }
    assert_eq!(
        format!("{:?}", SecretRef::Literal("hidden".into())),
        "SecretRef(<redacted>)"
    );
}

#[test]
fn relay_urls_normalize_without_losing_paths() {
    for (input, want) in [
        ("https://buzz.example.com", "wss://buzz.example.com"),
        ("https://buzz.example.com/", "wss://buzz.example.com"),
        ("http://localhost:3000", "ws://localhost:3000"),
        ("buzz.example.com", "wss://buzz.example.com"),
        ("wss://buzz.example.com", "wss://buzz.example.com"),
        ("ws://localhost:3000/", "ws://localhost:3000"),
        (
            "https://buzz.example.com/team/",
            "wss://buzz.example.com/team/",
        ),
        (
            "https://buzz.example.com/?community=one",
            "wss://buzz.example.com/?community=one",
        ),
    ] {
        assert_eq!(relay_url(input).expect(input), want);
    }
    for bad in [
        "",
        "  ",
        "ftp://buzz.example.com",
        "https://",
        "wss://user:pass@relay.example",
        "wss://relay.example/#part",
    ] {
        assert!(relay_url(bad).is_err(), "{bad} should be rejected");
    }
}
