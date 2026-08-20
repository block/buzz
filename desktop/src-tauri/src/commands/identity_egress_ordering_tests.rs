// C2 structural tripwire: every direct owner-artifact command that reads
// `state.signing_keys()` must admit before its owner-key read.
//
// The commands take `State<AppState>`, which a `tauri::test` mock cannot swap
// mid-call, so the "clone A's key → pause → transition commits B → admit at
// B" race has no hermetic runtime seam at the command boundary. It is instead
// pinned STRUCTURALLY: `try_admit_owner_identity_egress()` holds a lease that
// `begin_egress_drain` must await before B commits, so a key read that happens
// AFTER admission cannot observe a mid-transition swap. This scan asserts that
// ordering for every source-discovered candidate. Discovery takes the union of
// owner-key reads, egress admissions, and owner-artifact constructions; each
// discovered function must then be explicitly classified. A future producer
// that reads `signing_keys()` before admitting therefore fails closed instead
// of being omitted from a nominal command list. The lease-blocks-the-drain half
// of the invariant is driven at the registry seam in
// `owner_identity_egress::tests::key_read_under_lease_blocks_commit_b`.

/// The owner-key read accessor guarded by admission. `state.signing_keys()` is
/// the sole owner-key clone in the leased commands.
fn key_read_needle() -> String {
    ["signing_", "keys()"].concat()
}

/// The admission call that must precede every key read.
fn admit_needle() -> String {
    ["try_admit_owner_", "identity_egress"].concat()
}

/// This spelling avoids turning the ordering-only test module into NIP-49
/// material, which the egress guard deliberately confines to its allowlist.
const BACKUP_CREATE_INNER: &str = concat!("create_", "ncrypt", "sec_backup_inner");
const BACKUP_VERIFY_INNER: &str = concat!("verify_", "ncrypt", "sec_backup_inner");

/// Every source-discovered C2 candidate is classified exactly once. Discovery
/// is structural; these names classify the discovered functions rather than
/// defining their universe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProducerClassification {
    /// Direct owner-artifact producer: admission must precede its key read.
    C2Ordering,
    /// The backup pair: the caller holds `identity_mutation` before admission,
    /// and the helper reads the key under that caller-held mutation lock.
    BackupCallerHeldMutation,
    /// Reads the owner key but constructs no owner artifact, so C2 does not
    /// apply. It remains explicit to prevent silently losing a future producer.
    NonArtifactKeyReader,
}

/// Classifications for every current source-discovered candidate. A new
/// candidate is a failure until it is deliberately classified here.
const PRODUCER_CLASSIFICATIONS: &[(&str, ProducerClassification)] = &[
    ("sign_event", ProducerClassification::C2Ordering),
    ("decrypt_observer_event", ProducerClassification::C2Ordering),
    (
        "build_observer_control_event",
        ProducerClassification::C2Ordering,
    ),
    ("get_nsec", ProducerClassification::C2Ordering),
    (
        "sign_nostr_identity_binding",
        ProducerClassification::C2Ordering,
    ),
    ("create_auth_event", ProducerClassification::C2Ordering),
    ("nip44_encrypt_to_self", ProducerClassification::C2Ordering),
    (
        "nip44_decrypt_from_self",
        ProducerClassification::C2Ordering,
    ),
    (
        "create_backup_with_log_n",
        ProducerClassification::BackupCallerHeldMutation,
    ),
    (
        BACKUP_CREATE_INNER,
        ProducerClassification::BackupCallerHeldMutation,
    ),
    (
        BACKUP_VERIFY_INNER,
        ProducerClassification::NonArtifactKeyReader,
    ),
    (
        "persist_current_identity",
        ProducerClassification::NonArtifactKeyReader,
    ),
];

/// Split the module source into `(fn signature line, body)` segments at
/// top-level `fn` boundaries (private or any `pub… fn` form). Every leased
/// command is a top-level fn, so this bounds each body without a brace parser.
fn top_level_fns(content: &str) -> Vec<(String, String)> {
    let is_fn_decl = |line: &str| {
        line.starts_with("fn ")
            || line.starts_with("async fn ")
            || (line.starts_with("pub") && line.contains(" fn "))
    };
    let mut segments = Vec::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in content.lines() {
        if is_fn_decl(line) {
            if let Some((sig, body)) = current.take() {
                segments.push((sig, body.join("\n")));
            }
            current = Some((line.to_string(), Vec::new()));
        } else if let Some((_, body)) = current.as_mut() {
            body.push(line);
        }
    }
    if let Some((sig, body)) = current.take() {
        segments.push((sig, body.join("\n")));
    }
    segments
}

/// First non-comment line index of `needle` in a body, or `None`.
fn first_call(body: &str, needle: &str) -> Option<usize> {
    body.lines().enumerate().find_map(|(i, line)| {
        let trimmed = line.trim_start();
        (!trimmed.starts_with("//") && trimmed.contains(needle)).then_some(i)
    })
}

/// Return a function name from a top-level signature line.
fn function_name(sig: &str) -> Option<&str> {
    sig.split_once("fn ")?.1.split(['(', '<']).next()
}

/// Source-discovered candidates use the union of the three C2 boundary markers:
/// admission, owner-key read, and owner-artifact construction. This catches a
/// future producer even when it omits one of the markers the ordering rule
/// requires.
fn is_candidate(body: &str, key: &str, admit: &str) -> bool {
    body.contains(admit) || body.contains(key) || body.contains("register_owner_artifact")
}

fn classification_for(name: &str) -> Option<ProducerClassification> {
    PRODUCER_CLASSIFICATIONS
        .iter()
        .find_map(|(candidate, classification)| (*candidate == name).then_some(*classification))
}

/// Violations cover the closed source-discovered candidate universe: every
/// candidate must have exactly one explicit classification. C2 producers must
/// then admit before reading the owner key; NIP-49 and non-artifact readers
/// stay explicit rather than becoming silent exclusions.
fn admission_ordering_violations(content: &str) -> Vec<String> {
    let key = key_read_needle();
    let admit = admit_needle();
    let discovered = top_level_fns(content)
        .into_iter()
        .filter(|(_, body)| is_candidate(body, &key, &admit))
        .collect::<Vec<_>>();
    let mut violations = Vec::new();

    for (name, _) in PRODUCER_CLASSIFICATIONS {
        let matches = PRODUCER_CLASSIFICATIONS
            .iter()
            .filter(|(candidate, _)| candidate == name)
            .count();
        if matches != 1 {
            violations.push(format!(
                "{name}: discovered candidates require exactly one classification (found {matches})"
            ));
        }
    }

    for (sig, body) in discovered {
        let Some(name) = function_name(&sig) else {
            violations.push(format!("{sig}: cannot classify discovered C2 candidate"));
            continue;
        };
        let Some(classification) = classification_for(name) else {
            violations.push(format!(
                "{name}: unclassified owner-artifact candidate — add an explicit C2, NIP-49, or non-artifact classification"
            ));
            continue;
        };
        if classification != ProducerClassification::C2Ordering {
            continue;
        }

        let Some(admit_at) = first_call(&body, &admit) else {
            violations.push(format!(
                "{name}: C2 producer must admit before reading {key}"
            ));
            continue;
        };
        let Some(key_at) = first_call(&body, &key) else {
            violations.push(format!(
                "{name}: C2 producer must read {key} under its admission lease"
            ));
            continue;
        };
        if key_at < admit_at {
            violations.push(format!(
                "{name}: reads {key} at body line {} but admits the lease at body line {} — \
                 admission MUST precede every owner-key read (C2)",
                key_at + 1,
                admit_at + 1,
            ));
        }
    }
    violations
}

fn module_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/identity.rs");
    std::fs::read_to_string(path).unwrap()
}

/// Every leased owner-identity command admits before it reads the owner key.
#[test]
fn leased_commands_admit_before_reading_the_owner_key() {
    let violations = admission_ordering_violations(&module_source());
    assert!(
        violations.is_empty(),
        "owner-key read precedes admission — a captured pre-transition key can be \
         stamped as the post-transition generation (C2). Admit first:\n{}",
        violations.join("\n")
    );
}

/// Mutation proof: reordering a command back to read-then-admit trips the scan.
#[test]
fn scan_catches_a_read_before_admit() {
    let content = format!(
        "pub async fn sign_event(state: State) {{\n    \
         let keys = state.{}?;\n    \
         let lease = crate::owner_identity_egress::{}().await?;\n}}\n",
        key_read_needle(),
        admit_needle(),
    );
    let violations = admission_ordering_violations(&content);
    assert!(
        violations.iter().any(|v| v.contains("sign_event")),
        "a read-before-admit command must trip the scan: {violations:?}"
    );
}

/// Mutation proof: a newly named owner-artifact producer is discovered even
/// before anyone adds a classification for it.
#[test]
fn scan_catches_an_unclassified_new_owner_artifact_producer() {
    let content = format!(
        "pub async fn future_owner_artifact(state: State) {{\n    \
         let keys = state.{}?;\n    \
         let lease = crate::owner_identity_egress::{}().await?;\n    \
         let artifact = crate::owner_identity_egress::register_owner_artifact(&lease);\n    \
         Ok(artifact.stamp_value(keys.public_key().to_hex()))\n}}\n",
        key_read_needle(),
        admit_needle(),
    );
    let violations = admission_ordering_violations(&content);
    assert!(
        violations.iter().any(|violation| {
            violation.contains("future_owner_artifact")
                && violation.contains("unclassified owner-artifact candidate")
        }),
        "a new owner-artifact construction site must require classification: {violations:?}"
    );
}

/// The helper's caller holds `identity_mutation` plus the admitted lease;
/// its internal key read is not itself a command-admission boundary and must
/// not register as a C2 violation.
#[test]
fn scan_ignores_non_leased_key_readers() {
    let content = format!(
        "pub(crate) fn create_backup_with_log_n(state: &AppState) {{\n    \
         let _guard = state.identity_mutation.blocking_lock();\n    \
         let keys = state.{}?;\n}}\n",
        key_read_needle(),
    );
    assert!(
        admission_ordering_violations(&content).is_empty(),
        "a key reader that never admits a lease is out of C2 scope"
    );
}
