use super::*;

fn spec_of(yaml: &str) -> Spec {
    SwarmFile::parse(yaml)
        .expect("parse")
        .specs()
        .expect("specs")
        .remove(0)
}

#[test]
fn agent_overrides_defaults_and_environment_values_merge_by_key() {
    let spec = spec_of(
        r#"
defaults:
  relays: [wss://shared.example]
  harness: buzz-acp
  restart: never
  env:
    SHARED: retained
    CREDENTIAL: {env: SHARED_CREDENTIAL}
agents:
  - name: helper
    relays: [wss://agent.example]
    restart: on-failure
    env:
      CREDENTIAL: {file: keys/credential}
      BUZZ_ACP_MCP_COMMAND: ''
"#,
    );
    assert_eq!(spec.relays, Some(vec!["wss://agent.example".to_owned()]));
    assert_eq!(spec.harness.as_deref(), Some("buzz-acp"));
    assert_eq!(spec.restart, Some(Restart::OnFailure));
    assert!(
        matches!(spec.env.get("CREDENTIAL"), Some(SecretRef::File(path)) if path == Path::new("keys/credential"))
    );
    let env = Env::default();
    assert_eq!(
        spec.env["SHARED"].resolve(&env).unwrap().expose(),
        "retained"
    );
    assert_eq!(
        spec.env["BUZZ_ACP_MCP_COMMAND"]
            .resolve(&env)
            .unwrap()
            .expose(),
        ""
    );
}

#[test]
fn rejects_unknown_fields_and_shared_identity() {
    for (yaml, message) in [
        ("agents: [{name: helper, typo: true}]", "typo"),
        ("agents: [{name: helper, provider: anything}]", "provider"),
        (
            "defaults: {name: helper}\nagents: []",
            "`defaults:` must not set `name`",
        ),
        (
            "defaults: {nsec: {env: KEY}}\nagents: []",
            "`defaults:` must not set `nsec`",
        ),
        (
            "defaults: {auth_tag: anything}\nagents: []",
            "`defaults:` must not set `auth_tag`",
        ),
        (
            "defaults: {enabled: true}\nagents: []",
            "`defaults:` must not set `enabled`",
        ),
    ] {
        let error = format!("{:#}", SwarmFile::parse(yaml).unwrap().specs().unwrap_err());
        assert!(error.contains(message), "{error}");
    }
    assert!(SwarmFile::parse("agent: []").is_err());
}

#[test]
fn secret_sources_preserve_values_and_do_not_expose_them_in_debug() {
    let env = Env::from_iter([("VALUE", "  exact value  ")]);
    for yaml in ["'  exact value  '", "{env: VALUE}"] {
        let source: SecretRef = serde_yaml::from_str(yaml).unwrap();
        let value = source.resolve(&env).unwrap();
        assert_eq!(value.expose(), "  exact value  ");
        assert!(!format!("{source:?} {value:?}").contains("exact value"));
    }
    let missing: SecretRef = serde_yaml::from_str("{env: MISSING}").unwrap();
    assert!(format!("{:#}", missing.resolve(&env).unwrap_err()).contains("MISSING"));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("key");
    std::fs::write(&path, "  credential\n").unwrap();
    assert_eq!(
        SecretRef::File(path).resolve(&env).unwrap().expose(),
        "credential"
    );
}

#[test]
fn secret_sources_reject_ambiguous_or_unknown_forms() {
    for yaml in ["{env: A, file: key}", "{}", "{fiel: key}"] {
        assert!(serde_yaml::from_str::<SecretRef>(yaml).is_err(), "{yaml}");
    }
}

#[test]
fn paths_resolve_from_the_config_directory() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("swarm.yaml");
    std::fs::write(
        &path,
        r#"
owner: {nsec: {file: owner.nsec}}
defaults:
  harness: ./bin/harness
  env:
    CREDENTIAL: {file: secret}
agents:
  - name: helper
    nsec: {file: keys/helper.nsec}
    auth_tag: {file: auth.json}
  - name: other
    workdir: checkout
    harness: buzz-acp
"#,
    )
    .unwrap();
    let file = SwarmFile::load(&path).unwrap();
    let env = Env::default();
    assert!(
        matches!(file.owner(&env).unwrap().nsec, SecretRef::File(path) if path == directory.path().join("owner.nsec"))
    );
    let specs = file.resolved_specs(&env).unwrap();
    assert_eq!(
        specs[0].workdir, None,
        "the plan owns the per-agent default"
    );
    assert_eq!(
        Path::new(specs[0].harness.as_deref().unwrap()),
        directory.path().join("bin/harness")
    );
    for (source, relative) in [
        (specs[0].nsec.as_ref().unwrap(), "keys/helper.nsec"),
        (specs[0].auth_tag.as_ref().unwrap(), "auth.json"),
        (&specs[0].env["CREDENTIAL"], "secret"),
    ] {
        assert!(
            matches!(source, SecretRef::File(path) if path == &directory.path().join(relative))
        );
    }
    assert_eq!(specs[1].workdir, Some(directory.path().join("checkout")));
    assert_eq!(specs[1].harness.as_deref(), Some("buzz-acp"));
}

#[test]
fn tilde_expands_only_at_the_root() {
    let env = Env::from_iter([("HOME", "/home/operator")]);
    assert_eq!(
        expand_tilde(Path::new("~/keys/a"), &env),
        PathBuf::from("/home/operator/keys/a")
    );
    assert_eq!(
        expand_tilde(Path::new("/etc/~/a"), &env),
        PathBuf::from("/etc/~/a")
    );
    assert_eq!(
        expand_tilde(Path::new("~/a"), &Env::default()),
        PathBuf::from("~/a")
    );
}
