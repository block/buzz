use super::test_support::*;
use super::*;

// ── redaction ────────────────────────────────────────────────────────────────

/// Secrets that an installer echoed must not land on disk. The log is written
/// unattended, so scrubbing happens at the write, not at the read.
#[test]
fn test_log_redacts_secrets_before_writing() {
    let h = harness();
    let leak = "npm ERR! token nsec1qqqqqqqqqqsecretvalue failed";

    h.reporter.record_attempt(1, outcome("cli", false, leak));

    let log = h.log_contents();
    assert!(!log.contains("nsec1qqqqqqqqqqsecretvalue"), "got: {log}");
    assert!(log.contains("[REDACTED]"), "got: {log}");
}

/// The environment's own secrets are scrubbed too, by *name* rather than shape.
/// An install inherits Buzz's environment and installers echo it back — npm
/// prints its resolved config on an auth failure — and a token with no
/// recognizable prefix would otherwise reach the file verbatim.
#[test]
fn test_log_redacts_an_environment_secret_with_no_recognizable_prefix() {
    let secret = "0e8f31c5a4b7d296e5f1a";
    // Set before the reporter is built: the snapshot is taken at construction.
    std::env::set_var("BUZZ_TEST_REGISTRY_TOKEN", secret);
    let h = harness();
    std::env::remove_var("BUZZ_TEST_REGISTRY_TOKEN");

    h.reporter.record_attempt(
        1,
        outcome("cli", false, &format!("npm ERR! _authToken={secret}")),
    );

    let log = h.log_contents();
    assert!(!log.contains(secret), "got: {log}");
    assert!(log.contains("[REDACTED]"), "got: {log}");
}

/// A live line carries the same scrubbing as the log record. The line is
/// rendered verbatim in the UI, so a leak there is as visible as one on disk.
#[test]
fn test_a_live_line_is_redacted_before_it_is_emitted() {
    let h = harness();

    let observer = h.reporter.line_observer().expect("an observer");
    observer("fetching with token nsec1qqqqqqqqqqleaked");

    let lines = h.lines();
    assert_eq!(lines.len(), 1);
    let line = lines[0].clone().expect("a line, not a clear signal");
    assert!(!line.contains("nsec1qqqqqqqqqqleaked"), "got: {line}");
    assert!(line.contains("[REDACTED]"), "got: {line}");
}

// ── proxy and PAT credentials ────────────────────────────────────────────────

/// A proxy URL's password is a credential, but the proxy itself is diagnostic
/// information: an install that fails behind a proxy is only debuggable if the
/// record still says which proxy it went through. So the userinfo is scrubbed
/// and the host is kept.
#[test]
fn test_proxy_userinfo_is_secret_but_the_proxy_host_is_not() {
    let secrets = secret_values_from([(
        "HTTPS_PROXY".to_string(),
        "http://corpuser:hunter2pass@proxy.example:8080".to_string(),
    )]);

    assert_eq!(secrets, vec!["corpuser:hunter2pass"]);
}

/// A proxy with no credential contributes nothing — scrubbing a bare host would
/// erase the proxy's name from every record while protecting nothing. A bare
/// username is not a credential either, and scrubbing it would delete every
/// occurrence of that word from the log.
#[test]
fn test_a_proxy_without_credentials_contributes_no_secret() {
    let secrets = secret_values_from([
        (
            "HTTP_PROXY".to_string(),
            "http://proxy.example:8080".to_string(),
        ),
        (
            "ALL_PROXY".to_string(),
            "socks5://10.0.0.1:1080".to_string(),
        ),
        (
            "HTTPS_PROXY".to_string(),
            "http://user@proxy.example:8080".to_string(),
        ),
    ]);

    assert!(secrets.is_empty(), "got: {secrets:?}");
}

/// npm reads its own `npm_config_*` aliases in preference to the conventional
/// proxy variables and prints the resolved value back, so a credential set only
/// under an alias would otherwise never enter the scrub list. npm spells them
/// in lowercase, so both cases have to classify.
#[test]
fn test_npm_proxy_aliases_are_classified_in_either_case() {
    let secrets = secret_values_from([
        (
            "npm_config_proxy".to_string(),
            "http://corpuser:lowerplain@proxy.example:8080".to_string(),
        ),
        (
            "NPM_CONFIG_PROXY".to_string(),
            "http://corpuser:upperplain@proxy.example:8080".to_string(),
        ),
        (
            "npm_config_https_proxy".to_string(),
            "http://corpuser:lowertls@proxy.example:8080".to_string(),
        ),
        (
            "NPM_CONFIG_HTTPS_PROXY".to_string(),
            "http://corpuser:uppertls@proxy.example:8080".to_string(),
        ),
    ]);

    assert_eq!(
        secrets,
        vec![
            "corpuser:lowerplain",
            "corpuser:upperplain",
            "corpuser:lowertls",
            "corpuser:uppertls",
        ]
    );
}

/// The alias carries the same userinfo-only policy as the conventional names:
/// a credential-less alias contributes nothing, so the proxy stays named in the
/// record.
#[test]
fn test_an_npm_proxy_alias_without_credentials_contributes_no_secret() {
    let secrets = secret_values_from([
        (
            "npm_config_proxy".to_string(),
            "http://proxy.example:8080".to_string(),
        ),
        (
            "NPM_CONFIG_HTTPS_PROXY".to_string(),
            "http://user@proxy.example:8080".to_string(),
        ),
    ]);

    assert!(secrets.is_empty(), "got: {secrets:?}");
}

/// The classifier and the reporter have to agree: an alias credential that
/// classifies but never reaches the scrub list still leaks. This drives the
/// reporter with exactly what the classifier produced for an alias, and asserts
/// the log and the returned step both come back clean.
#[test]
fn test_an_npm_alias_credential_is_redacted_from_the_log_and_the_returned_step() {
    let password = "hunter2pass";
    let h = harness_with_secrets(secret_values_from([(
        "npm_config_proxy".to_string(),
        format!("http://corpuser:{password}@proxy.example:8080"),
    )]));

    let returned = h.reporter.record_attempt(
        1,
        InstallOutcome {
            step: InstallStepResult {
                stderr: format!(
                    "npm ERR! proxy=http://corpuser:{password}@proxy.example:8080 tunneling failed"
                ),
                ..step("cli", false, "")
            },
            log_stdout: String::new(),
            log_stderr: format!(
                "npm config: proxy = http://corpuser:{password}@proxy.example:8080"
            ),
        },
    );

    let log = h.log_contents();
    assert!(!log.contains(password), "log leaked the password: {log}");
    assert!(
        log.contains("proxy.example"),
        "the proxy host is diagnostic and must survive: {log}"
    );
    assert!(
        !returned.stderr.contains(password),
        "the returned step leaked the password: {}",
        returned.stderr
    );
}

// ── npm's own credential settings ────────────────────────────────────────────

/// npm accepts every one of its settings as an `npm_config_*` variable, so a
/// registry client key, a basic-auth blob or a one-time password can arrive
/// under a name that carries no marker. Their whole value is the credential —
/// unlike a proxy, none of it is diagnostic — and npm spells them in lowercase.
#[test]
fn test_npm_credential_configs_are_secret_in_either_case() {
    let secrets = secret_values_from([
        (
            "npm_config_key".to_string(),
            "-----BEGIN PRIVATE KEY-----lowerkey".to_string(),
        ),
        (
            "NPM_CONFIG_KEY".to_string(),
            "-----BEGIN PRIVATE KEY-----upperkey".to_string(),
        ),
        ("npm_config__auth".to_string(), "bG93ZXJhdXRo".to_string()),
        ("NPM_CONFIG__AUTH".to_string(), "dXBwZXJhdXRo".to_string()),
        ("npm_config_otp".to_string(), "618243".to_string()),
        ("NPM_CONFIG_OTP".to_string(), "907154".to_string()),
    ]);

    assert_eq!(
        secrets,
        vec![
            "-----BEGIN PRIVATE KEY-----lowerkey",
            "-----BEGIN PRIVATE KEY-----upperkey",
            "bG93ZXJhdXRo",
            "dXBwZXJhdXRo",
            "618243",
            "907154",
        ]
    );
}

/// An unset-but-exported credential is empty, and an empty needle would match
/// everywhere. The name being exact does not make a blank value a secret.
#[test]
fn test_an_empty_npm_credential_config_contributes_no_secret() {
    let secrets = secret_values_from([
        ("NPM_CONFIG_KEY".to_string(), String::new()),
        ("npm_config_otp".to_string(), String::new()),
    ]);

    assert!(secrets.is_empty(), "got: {secrets:?}");
}

/// A private registry's URL follows the proxy policy rather than the whole-value
/// one: which registry an install talked to is exactly what a 401 or an ETIMEDOUT
/// has to be read against, so only the userinfo is the secret.
#[test]
fn test_npm_registry_userinfo_is_secret_but_the_registry_host_is_not() {
    let secrets = secret_values_from([(
        "npm_config_registry".to_string(),
        "https://builder:hunter2pass@registry.example/api/npm/".to_string(),
    )]);

    assert_eq!(secrets, vec!["builder:hunter2pass"]);
}

/// The public registry — and any private one reached with a token header rather
/// than URL credentials — contributes nothing, so the registry stays named in
/// the record.
#[test]
fn test_an_npm_registry_without_credentials_contributes_no_secret() {
    let secrets = secret_values_from([
        (
            "npm_config_registry".to_string(),
            "https://registry.npmjs.org/".to_string(),
        ),
        (
            "NPM_CONFIG_REGISTRY".to_string(),
            "https://builder@registry.example/api/npm/".to_string(),
        ),
    ]);

    assert!(secrets.is_empty(), "got: {secrets:?}");
}

/// The credential settings are matched by exact name, never by a `KEY` or
/// `AUTH` substring. Those occur throughout an ordinary environment on values
/// that are paths, agent sockets and people's names, and scrubbing them would
/// delete unrelated text from every record.
#[test]
fn test_key_and_auth_inside_a_variable_name_do_not_make_it_secret() {
    let secrets = secret_values_from([
        (
            "SSH_AUTH_SOCK".to_string(),
            "/tmp/ssh-agent.socket".to_string(),
        ),
        ("GIT_AUTHOR_NAME".to_string(), "Ada Lovelace".to_string()),
        (
            "KEYCHAIN".to_string(),
            "/Users/dev/Library/login.keychain".to_string(),
        ),
        (
            "NPM_CONFIG_KEYFILE".to_string(),
            "/Users/dev/.npm/client.pem".to_string(),
        ),
    ]);

    assert!(secrets.is_empty(), "got: {secrets:?}");
}

/// The wiring, not just the classification: npm prints its resolved config on an
/// auth failure, so each of these has to be gone from the log and from the step
/// the frontend renders. The one-time password is the interesting one — at six
/// digits it is far shorter than any other secret here, and a value under four
/// bytes is dropped by the shared redactor rather than scrubbed.
#[test]
fn test_npm_credential_configs_are_redacted_from_the_log_and_the_returned_step() {
    let client_key = "-----BEGIN PRIVATE KEY-----MIIEvQIBADAN";
    let auth = "YnVpbGRlcjpodW50ZXIycGFzcw==";
    let otp = "618243";
    let registry_password = "hunter2pass";
    let h = harness_with_secrets(secret_values_from([
        ("npm_config_key".to_string(), client_key.to_string()),
        ("npm_config__auth".to_string(), auth.to_string()),
        ("npm_config_otp".to_string(), otp.to_string()),
        (
            "npm_config_registry".to_string(),
            format!("https://builder:{registry_password}@registry.example/api/npm/"),
        ),
    ]));

    let returned = h.reporter.record_attempt(
        1,
        InstallOutcome {
            step: InstallStepResult {
                stderr: format!("npm ERR! 401 otp={otp} _auth={auth}"),
                ..step("cli", false, "")
            },
            log_stdout: format!("npm config: key = {client_key}"),
            log_stderr: format!(
                "npm config: registry = https://builder:{registry_password}@registry.example/api/npm/"
            ),
        },
    );

    let log = h.log_contents();
    for secret in [client_key, auth, otp, registry_password] {
        assert!(!log.contains(secret), "log leaked {secret}: {log}");
    }
    assert!(
        log.contains("registry.example"),
        "the registry host is diagnostic and must survive: {log}"
    );
    assert!(
        !returned.stderr.contains(otp) && !returned.stderr.contains(auth),
        "the returned step leaked a credential: {}",
        returned.stderr
    );
}

/// `*_PATH` variables must not be mistaken for personal access tokens. A
/// `contains("_PAT")` rule would match `PATH` itself and scrub every directory
/// name out of the log, which is why the rule matches `_PAT` as a suffix.
#[test]
fn test_a_path_variable_is_not_treated_as_a_personal_access_token() {
    let secrets = secret_values_from([
        ("PATH".to_string(), "/usr/local/bin:/usr/bin".to_string()),
        ("GOPATH".to_string(), "/home/user/go".to_string()),
        (
            "CARGO_HOME_PATH".to_string(),
            "/home/user/.cargo".to_string(),
        ),
    ]);

    assert!(secrets.is_empty(), "got: {secrets:?}");
}

/// Variables named as personal access tokens are secret by name, whatever shape
/// their value has.
#[test]
fn test_pat_named_variables_are_secret() {
    let secrets = secret_values_from([
        (
            "GITHUB_PAT".to_string(),
            "ghp_abcdefghij0123456789".to_string(),
        ),
        (
            "GH_PAT".to_string(),
            "github_pat_abcdefghij0123".to_string(),
        ),
    ]);

    assert_eq!(secrets.len(), 2, "got: {secrets:?}");
}

/// The whole point of the widening: a proxy password and a PAT that the
/// installer echoed reach neither the log nor the live line.
///
/// Both are checked through the real reporter rather than the classifier, so
/// this covers the wiring — a classifier that recognises a secret the reporter
/// never consults would still leak.
#[test]
fn test_proxy_and_pat_credentials_are_redacted_from_the_log_and_the_live_line() {
    let proxy_password = "hunter2pass";
    let pat = "ghp_abcdefghij0123456789";
    // The classifier's own tests cover recognising these under their real
    // variable names; injecting the resulting secrets here keeps a live
    // `HTTPS_PROXY` out of the process the rest of the suite shares.
    let h = harness_with_secrets(vec![format!("corpuser:{proxy_password}"), pat.to_string()]);

    h.reporter.record_attempt(
        1,
        outcome(
            "cli",
            false,
            &format!(
                "npm ERR! proxy=http://corpuser:{proxy_password}@proxy.example authToken={pat}"
            ),
        ),
    );
    let observer = h.reporter.line_observer().expect("an observer");
    observer(&format!("cloning https://{pat}@github.com/org/repo"));

    let log = h.log_contents();
    assert!(
        !log.contains(proxy_password),
        "log leaked the proxy password: {log}"
    );
    assert!(!log.contains(pat), "log leaked the PAT: {log}");
    assert!(
        log.contains("proxy.example"),
        "the proxy host is diagnostic and must survive: {log}"
    );

    let line = h.lines().into_iter().flatten().next().expect("a live line");
    assert!(!line.contains(pat), "live line leaked the PAT: {line}");
    assert!(line.contains("[REDACTED]"), "got: {line}");
}

/// The third surface: the step returned to the frontend. `getInstallErrorMessage`
/// renders the failing step's stderr verbatim, so a secret that the log and the
/// live line both scrub would still reach the user through the error dialog.
#[test]
fn test_a_returned_step_is_redacted_before_the_frontend_renders_it() {
    let pat = "ghp_abcdefghij0123456789";
    let h = harness_with_secrets(vec![pat.to_string()]);

    let returned = h.reporter.record_attempt(
        1,
        InstallOutcome {
            step: InstallStepResult {
                stdout: format!("configuring remote with {pat}"),
                stderr: format!("fatal: authentication failed for token {pat}"),
                hint: Some(format!("check that {pat} has the repo scope")),
                ..step("cli", false, "")
            },
            log_stdout: String::new(),
            log_stderr: String::new(),
        },
    );

    assert!(!returned.stdout.contains(pat), "got: {}", returned.stdout);
    assert!(!returned.stderr.contains(pat), "got: {}", returned.stderr);
    let hint = returned.hint.expect("a hint");
    assert!(!hint.contains(pat), "got: {hint}");
}

/// A synthesized step reaches the frontend through the other funnel, and needs
/// the same scrubbing — the managed-node prerequisite failures are built this
/// way and carry whatever the underlying command printed.
#[test]
fn test_a_synthesized_step_is_redacted_before_it_reaches_the_caller() {
    let pat = "ghp_abcdefghij0123456789";
    let h = harness_with_secrets(vec![pat.to_string()]);
    let mut steps = Vec::new();

    h.reporter.record_step(
        &mut steps,
        InstallStepResult {
            stderr: format!("npm ERR! 401 with {pat}"),
            ..step("adapter", false, "")
        },
    );

    assert_eq!(steps.len(), 1);
    assert!(!steps[0].stderr.contains(pat), "got: {}", steps[0].stderr);
    assert!(
        steps[0].stderr.contains("[REDACTED]"),
        "got: {}",
        steps[0].stderr
    );
}
