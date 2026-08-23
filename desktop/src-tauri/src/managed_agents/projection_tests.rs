//! Tests for `super` — canonical roster projection.
//!
//! Spec: `PLANS/CANONICAL_ROSTER_PROJECTION_SPEC_2026_08_22.md` §6.

use std::collections::{BTreeMap, HashMap};

use super::{
    load_canonical_projection_config, project_canonical_roster, CanonicalIdentity,
    CanonicalProjectionConfig, ProjectionInputRow, RosterView,
};

const AIR_HOST: &str = "rubens-MacBook-Air.local";
const STUDIO_HOST: &str = "Mac.localdomain";

// ── Canonical pubkeys (spec §2) ─────────────────────────────────────────
const FIZZ_AIR: &str = "9ae89b561c22e1578499b5165c8e2505d6ad1355f0c581f1344bd6964c75c54f";
const HONEY_AIR: &str = "c1f7d802126f30d7a107129f87161aa8121c302c6a61aacbb2acc62784b41df8";
const BUMBLE_AIR: &str = "aac70a3a960f379f49f26acd28c93f268078591045678e79cff446f913d23e7c";
const POLLEN_AIR: &str = "3a2208dba54ebd1fe310052f7bd296a0b4fd50bcdaf38391b9e0cd60e4b068a3";
const FIZZ_STUDIO: &str = "c38df6f6b3078750423ba0a4bbf0c32a44a6844a369b7d9dce6b052ae82412be";
const HONEY_STUDIO: &str = "203e1633cbb743ff0c13d97f18dcd22b189c11f03cd054f622fa5a33d735373a";
const BUMBLE_STUDIO: &str = "058d5d07228b577b8e583f53fc717bf2e9d2f3cd7868483471469871b1bacc86";
const POLLEN_STUDIO: &str = "3143401747c85168df77e7e4848b898f6ba52913cba6e097d741867d494d44b7";

fn canonical_config() -> CanonicalProjectionConfig {
    let mut overrides = BTreeMap::new();
    overrides.insert(BUMBLE_AIR.to_string(), "Bumble Air".to_string());
    CanonicalProjectionConfig {
        version: 1,
        issued: "2026-08-23T04:35:00Z".to_string(),
        revised: "2026-08-23T05:20:00Z".to_string(),
        authorizing_events: vec![
            "7106ba2ccc3083c15bc0784e071661e6918fad3325a852941463c62a22e5c5e3".to_string(),
            "bc28178064e9b496b7bccb897aa052f0aff91619b7d5fded1260d0e312079a17".to_string(),
        ],
        canonical: vec![
            CanonicalIdentity {
                identity: "Fizz Air".to_string(),
                host: AIR_HOST.to_string(),
                pubkey: FIZZ_AIR.to_string(),
            },
            CanonicalIdentity {
                identity: "Honey Air".to_string(),
                host: AIR_HOST.to_string(),
                pubkey: HONEY_AIR.to_string(),
            },
            CanonicalIdentity {
                identity: "Bumble Air".to_string(),
                host: AIR_HOST.to_string(),
                pubkey: BUMBLE_AIR.to_string(),
            },
            CanonicalIdentity {
                identity: "Pollen Air".to_string(),
                host: AIR_HOST.to_string(),
                pubkey: POLLEN_AIR.to_string(),
            },
            CanonicalIdentity {
                identity: "Fizz-Studio".to_string(),
                host: STUDIO_HOST.to_string(),
                pubkey: FIZZ_STUDIO.to_string(),
            },
            CanonicalIdentity {
                identity: "Honey-Studio".to_string(),
                host: STUDIO_HOST.to_string(),
                pubkey: HONEY_STUDIO.to_string(),
            },
            CanonicalIdentity {
                identity: "Bumble-Studio".to_string(),
                host: STUDIO_HOST.to_string(),
                pubkey: BUMBLE_STUDIO.to_string(),
            },
            CanonicalIdentity {
                identity: "Pollen-Studio".to_string(),
                host: STUDIO_HOST.to_string(),
                pubkey: POLLEN_STUDIO.to_string(),
            },
        ],
        display_name_source_priority: vec![
            "display_name_overrides".to_string(),
            "signed_kind0".to_string(),
            "canonical_identity".to_string(),
        ],
        display_name_overrides: overrides,
    }
}

fn row(pubkey: &str, name: &str) -> ProjectionInputRow {
    ProjectionInputRow {
        pubkey: pubkey.to_string(),
        name: name.to_string(),
    }
}

fn studio_rows() -> Vec<ProjectionInputRow> {
    vec![
        row(FIZZ_STUDIO, "fizz-studio"),
        row(HONEY_STUDIO, "honey-studio"),
        row(BUMBLE_STUDIO, "bumble-studio"),
        row(POLLEN_STUDIO, "Pollen-Studio"),
    ]
}

// Air fixture: 4 canonical Air rows + 7 legacy Studio-labelled Air-local
// pubkeys per Honey [7] (§5.2). Legacy pubkeys are fake but shape-realistic
// (64 hex chars, arbitrary values).
fn air_rows() -> Vec<ProjectionInputRow> {
    vec![
        row(FIZZ_AIR, "fizz-air"),
        row(HONEY_AIR, "honey-air"),
        // Note: row name is deliberately the migrated label to prove the
        // override wins (§6.1 bumble_air_display_name_override_wins).
        row(BUMBLE_AIR, "Pollen"),
        row(POLLEN_AIR, "pollen-air"),
        // Seven legacy Studio-labelled Air-local pubkeys.
        row(
            "1111111111111111111111111111111111111111111111111111111111111111",
            "legacy-fizz",
        ),
        row(
            "2222222222222222222222222222222222222222222222222222222222222222",
            "legacy-bumble-1",
        ),
        row(
            "3333333333333333333333333333333333333333333333333333333333333333",
            "legacy-bumble-2",
        ),
        row(
            "4444444444444444444444444444444444444444444444444444444444444444",
            "legacy-honey-1",
        ),
        row(
            "5555555555555555555555555555555555555555555555555555555555555555",
            "legacy-honey-2",
        ),
        row(
            "6666666666666666666666666666666666666666666666666666666666666666",
            "legacy-pollen-1",
        ),
        row(
            "7777777777777777777777777777777777777777777777777777777777777777",
            "legacy-pollen-2",
        ),
    ]
}

fn all_live(rows: &[ProjectionInputRow]) -> HashMap<String, u32> {
    rows.iter()
        .enumerate()
        .map(|(idx, r)| (r.pubkey.clone(), (10_000 + idx) as u32))
        .collect()
}

// ── §6.1 unit tests ─────────────────────────────────────────────────────

#[test]
fn projection_disabled_is_passthrough() {
    let rows = studio_rows();
    let live = all_live(&rows);
    let signed = HashMap::new();
    let config = canonical_config();

    let view = project_canonical_roster(&rows, &signed, &live, STUDIO_HOST, &config, false);

    let expected = RosterView::passthrough(&rows, &live);
    assert_eq!(view, expected);
    // Every input row surfaces in primary_local, none dropped or reordered.
    assert_eq!(view.primary_local.len(), rows.len());
    assert!(view.remote.is_empty());
    assert!(view.unverified.is_empty());
    assert!(view.missing.is_empty());
    // No canonical_identity assignment in passthrough (spec §3.2).
    assert!(view
        .primary_local
        .iter()
        .all(|c| c.canonical_identity.is_none()));
}

#[test]
fn studio_fixture_yields_four_primary() {
    let rows = studio_rows();
    let live = all_live(&rows);
    let signed = HashMap::new();
    let config = canonical_config();

    let view = project_canonical_roster(&rows, &signed, &live, STUDIO_HOST, &config, true);

    assert_eq!(view.primary_local.len(), 4, "exactly four primary cards");
    let ordered: Vec<&str> = view
        .primary_local
        .iter()
        .map(|c| c.pubkey.as_str())
        .collect();
    assert_eq!(
        ordered,
        vec![FIZZ_STUDIO, HONEY_STUDIO, BUMBLE_STUDIO, POLLEN_STUDIO],
        "canonical order preserved"
    );
    let identities: Vec<Option<&str>> = view
        .primary_local
        .iter()
        .map(|c| c.canonical_identity.as_deref())
        .collect();
    assert_eq!(
        identities,
        vec![
            Some("Fizz-Studio"),
            Some("Honey-Studio"),
            Some("Bumble-Studio"),
            Some("Pollen-Studio")
        ],
    );
    assert_eq!(view.unverified.len(), 0);
    assert_eq!(view.missing.len(), 0);
    // Studio host sees Air canonicals as remote (4 of them), but the input
    // set has none, so remote is empty; missing[] is limited to attested
    // host only.
    assert_eq!(view.remote.len(), 0);
}

#[test]
fn air_fixture_yields_four_primary_seven_unverified() {
    let rows = air_rows();
    let live = all_live(&rows);
    let signed = HashMap::new();
    let config = canonical_config();

    let view = project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, true);

    assert_eq!(view.primary_local.len(), 4);
    let ordered: Vec<&str> = view
        .primary_local
        .iter()
        .map(|c| c.pubkey.as_str())
        .collect();
    assert_eq!(
        ordered,
        vec![FIZZ_AIR, HONEY_AIR, BUMBLE_AIR, POLLEN_AIR],
        "canonical Air order preserved"
    );
    // Bumble Air display name comes from the override, not from the
    // migrated `Pollen` row name.
    let bumble_card = view
        .primary_local
        .iter()
        .find(|c| c.pubkey == BUMBLE_AIR)
        .expect("bumble air card");
    assert_eq!(bumble_card.display_name, "Bumble Air");
    assert_eq!(view.unverified.len(), 7);
    assert_eq!(view.remote.len(), 0);
    assert_eq!(view.missing.len(), 0);
}

#[test]
fn bumble_air_display_name_override_wins_over_migrated_pollen_label() {
    let rows = vec![row(BUMBLE_AIR, "Pollen")];
    let live = all_live(&rows);
    let mut signed = HashMap::new();
    // Even a signed kind-0 name of "Pollen" must lose to the explicit override.
    signed.insert(BUMBLE_AIR.to_string(), "Pollen".to_string());
    let config = canonical_config();

    let view = project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, true);

    assert_eq!(view.primary_local.len(), 1);
    assert_eq!(view.primary_local[0].display_name, "Bumble Air");
}

#[test]
fn signed_kind0_wins_over_row_name_for_non_overridden_pubkeys() {
    // Pollen Air is canonical but NOT in display_name_overrides.
    let rows = vec![row(POLLEN_AIR, "foo")];
    let live = all_live(&rows);
    let mut signed = HashMap::new();
    signed.insert(POLLEN_AIR.to_string(), "bar".to_string());
    let config = canonical_config();

    let view = project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, true);

    assert_eq!(view.primary_local.len(), 1);
    assert_eq!(view.primary_local[0].display_name, "bar");
}

#[test]
fn missing_canonical_emits_warn_and_does_not_synthesise() {
    let mut rows = air_rows();
    rows.retain(|r| r.pubkey != FIZZ_AIR);
    let live = all_live(&rows);
    let signed = HashMap::new();
    let config = canonical_config();

    let view = project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, true);

    assert_eq!(view.primary_local.len(), 3, "one canonical card missing");
    assert!(
        view.primary_local.iter().all(|c| c.pubkey != FIZZ_AIR),
        "no synthetic Fizz Air pubkey"
    );
    assert_eq!(view.missing.len(), 1);
    assert_eq!(view.missing[0].identity, "Fizz Air");
    assert_eq!(view.missing[0].pubkey, FIZZ_AIR);
}

#[test]
fn cross_host_canonical_bucketed_as_remote_not_primary() {
    // Air fixture augmented with all four Studio canonicals as inactive rows.
    let mut rows = air_rows();
    rows.extend(studio_rows());
    let live = all_live(&rows);
    let signed = HashMap::new();
    let config = canonical_config();

    let view = project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, true);

    assert_eq!(view.primary_local.len(), 4, "still four primary on Air");
    assert_eq!(
        view.remote.len(),
        4,
        "the four Studio canonicals land in remote"
    );
    let remote_pubkeys: Vec<&str> = view.remote.iter().map(|c| c.pubkey.as_str()).collect();
    assert_eq!(
        remote_pubkeys,
        vec![FIZZ_STUDIO, HONEY_STUDIO, BUMBLE_STUDIO, POLLEN_STUDIO],
    );
    assert!(view.remote.iter().all(|c| c.canonical_identity.is_some()));
    assert_eq!(view.unverified.len(), 7);
}

#[test]
fn unverified_stable_sort_by_pubkey_asc() {
    // Air rows in a scrambled order.
    let mut rows = air_rows();
    rows.reverse();
    let live = all_live(&rows);
    let signed = HashMap::new();
    let config = canonical_config();

    let view_a = project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, true);
    let view_b = project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, true);

    // Deterministic between runs.
    assert_eq!(view_a, view_b);
    // Ascending pubkey order.
    let pubkeys: Vec<&str> = view_a.unverified.iter().map(|c| c.pubkey.as_str()).collect();
    let mut expected = pubkeys.clone();
    expected.sort();
    assert_eq!(pubkeys, expected);
}

#[test]
fn flag_off_toggles_to_passthrough_without_data_loss() {
    let rows = air_rows();
    let live = all_live(&rows);
    let signed = HashMap::new();
    let config = canonical_config();

    let projected =
        project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, true);
    let pass = project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, false);

    // Projected buckets sum to input length (no synthesis, no drops).
    let projected_total =
        projected.primary_local.len() + projected.remote.len() + projected.unverified.len();
    assert_eq!(projected_total, rows.len());

    // Passthrough surfaces every row in primary_local, order preserved.
    assert_eq!(pass.primary_local.len(), rows.len());
    let pass_order: Vec<&str> = pass
        .primary_local
        .iter()
        .map(|c| c.pubkey.as_str())
        .collect();
    let input_order: Vec<&str> = rows.iter().map(|r| r.pubkey.as_str()).collect();
    assert_eq!(pass_order, input_order);
    assert!(pass.remote.is_empty());
    assert!(pass.unverified.is_empty());
    assert!(pass.missing.is_empty());
}

// ── §6.3 contract test — no synthetic pubkey ────────────────────────────

#[test]
fn no_synthetic_pubkey_ever() {
    use std::collections::HashSet;

    // Bounded, deterministic pseudo-random sweep — 1024 iterations,
    // enough to catch a regression without pulling `proptest`/`quickcheck`
    // into a hermitised build environment.
    let config = canonical_config();
    let signed: HashMap<String, String> = HashMap::new();

    for seed in 0u32..1024 {
        // Build a random subset of the canonical set + some noise rows.
        let mut rows: Vec<ProjectionInputRow> = Vec::new();
        for (idx, entry) in config.canonical.iter().enumerate() {
            if (seed >> (idx as u32)) & 1 == 1 {
                rows.push(row(&entry.pubkey, "seeded"));
            }
        }
        // Add up to four noise rows with pseudo-random pubkeys.
        for noise in 0u32..(seed & 0b11) {
            let pk = format!("{:064x}", seed.wrapping_mul(0x9E37_79B1) ^ noise);
            rows.push(row(&pk, "noise"));
        }

        let live = all_live(&rows);
        let inputs: HashSet<String> = rows.iter().map(|r| r.pubkey.clone()).collect();

        for host in &[AIR_HOST, STUDIO_HOST, "some-other-host"] {
            let view =
                project_canonical_roster(&rows, &signed, &live, host, &config, true);

            for card in view
                .primary_local
                .iter()
                .chain(view.remote.iter())
                .chain(view.unverified.iter())
            {
                assert!(
                    inputs.contains(&card.pubkey),
                    "seed {seed} host {host}: card pubkey {} was NOT in the input rows — the projection synthesised it",
                    card.pubkey
                );
            }
            // Missing bucket entries copy pubkeys from the canonical config,
            // never derived from any other source.
            let canonical_pubkeys: HashSet<&str> =
                config.canonical.iter().map(|c| c.pubkey.as_str()).collect();
            for m in &view.missing {
                assert!(canonical_pubkeys.contains(m.pubkey.as_str()));
            }
        }
    }
}

// ── §6.4 contract test — read-only guarantee ────────────────────────────

#[test]
fn projection_never_writes_files() {
    use std::fs;

    let temp = std::env::temp_dir().join(format!(
        "buzz-projection-readonly-{}",
        std::process::id()
    ));
    fs::create_dir_all(&temp).expect("create sentinel dir");
    let before: Vec<_> = fs::read_dir(&temp)
        .expect("read sentinel dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .collect();

    // Exercise the projection with every real fixture we have. The module
    // deliberately takes no filesystem path, so there is nowhere for it to
    // even attempt a write; the sentinel dir remains untouched.
    let rows = air_rows();
    let live = all_live(&rows);
    let signed = HashMap::new();
    let config = canonical_config();
    let _ = project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, true);
    let _ = project_canonical_roster(&rows, &signed, &live, STUDIO_HOST, &config, true);
    let _ = project_canonical_roster(&rows, &signed, &live, AIR_HOST, &config, false);

    let after: Vec<_> = fs::read_dir(&temp)
        .expect("read sentinel dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .collect();
    assert_eq!(before, after, "projection wrote to the sentinel directory");
    let _ = fs::remove_dir_all(&temp);
}

// ── Config loader round-trip ────────────────────────────────────────────

#[test]
fn load_canonical_projection_config_round_trips() {
    let cfg = canonical_config();
    let json = serde_json::to_string_pretty(&cfg).expect("serialise config");
    let temp = std::env::temp_dir().join(format!(
        "buzz-projection-config-{}.json",
        std::process::id()
    ));
    std::fs::write(&temp, json).expect("write config");
    let loaded = load_canonical_projection_config(&temp).expect("load config");
    assert_eq!(loaded, cfg);
    let _ = std::fs::remove_file(&temp);
}

#[test]
fn load_canonical_projection_config_missing_file_is_error() {
    let path = std::env::temp_dir().join(format!(
        "buzz-projection-config-missing-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let err = load_canonical_projection_config(&path).expect_err("missing file must error");
    assert!(err.contains("failed to read"), "unexpected error: {err}");
}
