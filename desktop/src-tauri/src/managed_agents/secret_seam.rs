//! Hydrate-on-load / strip-on-save seam for the generation-reference protocol.
//!
//! Each public function here is the single call site that moves secret values
//! (env vars, auth tags, provider configs) between in-memory records and the
//! OS keyring. `storage.rs` calls these at the top of every load path and at
//! the bottom of every save path.

use crate::managed_agents::{
    secret_projection::{
        agent_auth_tag_key, agent_env_key, agent_provider_config_key, cancel_gc_candidacy,
        definition_env_key, deserialize_env_map, deserialize_provider_config, load_secret,
        serialize_env_map, serialize_provider_config, write_secret, write_secrets_batched,
        FieldSave, ProjectionStore, WriteOutcome,
    },
    BackendKind, ManagedAgentRecord,
};

// ── Inline precedence rule (spec §1) ──────────────────────────────────────
//
//   - Non-empty env_vars / Some(auth_tag) / non-Null provider config →
//     AUTHORITATIVE (inline fallback state). Keyring ref is ignored on load.
//   - On save: success → clear inline, set ref, cancel candidacy. The old
//             generation is retired by the two-cycle GC, never eagerly here —
//             deleting before the JSON commit could orphan a committed secret.
//             failure → keep inline, clear ref (retry next boot).

/// Hydrate secret fields of an instance record from the keyring.
pub(crate) fn hydrate_agent_secrets_with<S: ProjectionStore>(
    store: &S,
    record: &mut ManagedAgentRecord,
) -> Vec<String> {
    let mut errors = Vec::new();
    if record.env_vars.is_empty() {
        match load_secret(
            store,
            record.env_vars_ref.as_deref(),
            |gen| agent_env_key(&record.pubkey, gen),
            &format!("agent:{} env_vars", record.pubkey),
        ) {
            Ok(Some(s)) => match deserialize_env_map(&s) {
                Ok(map) => record.env_vars = map,
                Err(e) => errors.push(format!(
                    "agent {} env_vars deserialization failed: {e}",
                    record.pubkey
                )),
            },
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
    }
    if record.auth_tag.is_none() {
        match load_secret(
            store,
            record.auth_tag_ref.as_deref(),
            |gen| agent_auth_tag_key(&record.pubkey, gen),
            &format!("agent:{} auth_tag", record.pubkey),
        ) {
            Ok(Some(v)) => record.auth_tag = Some(v),
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
    }
    if let BackendKind::Provider {
        ref id,
        ref mut config,
    } = record.backend
    {
        if config.is_null() {
            match load_secret(
                store,
                record.provider_config_ref.as_deref(),
                |gen| agent_provider_config_key(&record.pubkey, gen),
                &format!("agent:{} provider_config", record.pubkey),
            ) {
                Ok(Some(s)) => match deserialize_provider_config(&s) {
                    Ok(v) => *config = v,
                    Err(e) => errors.push(format!(
                        "agent {} provider_config deserialization failed: {e}",
                        record.pubkey
                    )),
                },
                Ok(None) => {}
                Err(e) => errors.push(e),
            }
        }
        let _ = id;
    }
    errors
}

/// Strip and persist secret fields of an instance record before JSON write.
///
/// # Fail-closed against empty-projection over unavailable
///
/// When `record.secrets_unavailable` is set (a `*_ref` failed to hydrate on
/// load, leaving the field empty in memory), a naive persist would call
/// `write_secret` with an empty value, get [`WriteOutcome::Nothing`], and
/// CLEAR the still-live `*_ref` — orphaning the generation permanently. On the
/// `Nothing` branch we therefore PRESERVE any pre-existing ref instead of
/// clearing it, so a transient keyring outage never destroys the pointer to a
/// live secret. Real user-cleared fields on an available record still clear
/// normally (`secrets_unavailable` is false).
pub(crate) fn strip_and_persist_agent_secrets_with<S: ProjectionStore>(
    store: &S,
    record: &mut ManagedAgentRecord,
) {
    let unavailable = record.secrets_unavailable;
    let pubkey = record.pubkey.clone();

    // Serialize each field's inline value once. `None` means "field empty" and
    // maps to WriteOutcome::Nothing (no write, no reuse).
    let inline_env = if !record.env_vars.is_empty() {
        serialize_env_map(&record.env_vars).ok()
    } else {
        None
    };
    let auth_val = record.auth_tag.clone();
    let provider = match &record.backend {
        BackendKind::Provider { id, config } => {
            let serialized = if !config.is_null() {
                serialize_provider_config(config).ok()
            } else {
                None
            };
            Some((id.clone(), serialized))
        }
        _ => None,
    };

    // Coordinate builders borrow `pubkey`; contexts are owned so they outlive
    // the batched call. Both are dropped before `record` is mutated below.
    let env_key = |gen: &str| agent_env_key(&pubkey, gen);
    let auth_key = |gen: &str| agent_auth_tag_key(&pubkey, gen);
    let pc_key = |gen: &str| agent_provider_config_key(&pubkey, gen);
    let env_ctx = format!("agent:{pubkey} env_vars");
    let auth_ctx = format!("agent:{pubkey} auth_tag");
    let pc_ctx = provider
        .as_ref()
        .map(|(id, _)| format!("agent:{pubkey} provider_config ({id})"));

    let mut fields = vec![
        FieldSave {
            coord_key_fn: &env_key,
            value: inline_env.as_deref(),
            existing_ref: record.env_vars_ref.as_deref(),
            context: &env_ctx,
        },
        FieldSave {
            coord_key_fn: &auth_key,
            value: auth_val.as_deref(),
            existing_ref: record.auth_tag_ref.as_deref(),
            context: &auth_ctx,
        },
    ];
    if let Some((_, serialized)) = &provider {
        fields.push(FieldSave {
            coord_key_fn: &pc_key,
            value: serialized.as_deref(),
            existing_ref: record.provider_config_ref.as_deref(),
            context: pc_ctx.as_deref().unwrap_or_default(),
        });
    }

    // ONE blob mutation for every changed field; unchanged fields reuse their
    // live generation and write nothing.
    let outcomes = write_secrets_batched(store, &fields);
    drop(fields); // end the immutable borrow of `record` before mutating it.

    match &outcomes[0] {
        WriteOutcome::Persisted { gen } => {
            record.env_vars.clear();
            record.env_vars_ref = Some(gen.clone());
        }
        // Empty projection: preserve an existing ref when unavailable so a
        // failed-hydrate save does not orphan the live generation.
        WriteOutcome::Nothing if unavailable => {}
        WriteOutcome::KeptInline { .. } | WriteOutcome::Nothing => {
            record.env_vars_ref = None;
        }
    }
    match &outcomes[1] {
        WriteOutcome::Persisted { gen } => {
            record.auth_tag = None;
            record.auth_tag_ref = Some(gen.clone());
        }
        WriteOutcome::Nothing if unavailable => {}
        WriteOutcome::KeptInline { .. } | WriteOutcome::Nothing => {
            record.auth_tag_ref = None;
        }
    }
    if provider.is_some() {
        match &outcomes[2] {
            WriteOutcome::Persisted { gen } => {
                if let BackendKind::Provider { config, .. } = &mut record.backend {
                    *config = serde_json::Value::Null;
                }
                record.provider_config_ref = Some(gen.clone());
            }
            WriteOutcome::Nothing if unavailable => {}
            WriteOutcome::KeptInline { .. } | WriteOutcome::Nothing => {
                record.provider_config_ref = None;
            }
        }
    } else {
        record.provider_config_ref = None; // Provider→Local: clear stale ref
    }
}

/// Hydrate secret fields of a definition (key-less) record.
pub(crate) fn hydrate_definition_secrets_with<S: ProjectionStore>(
    store: &S,
    record: &mut ManagedAgentRecord,
) -> Vec<String> {
    let Some(slug) = record.slug.as_deref().map(str::to_string) else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    if record.env_vars.is_empty() {
        match load_secret(
            store,
            record.env_vars_ref.as_deref(),
            |gen| definition_env_key(&slug, gen),
            &format!("definition:{slug} env_vars"),
        ) {
            Ok(Some(s)) => match deserialize_env_map(&s) {
                Ok(map) => record.env_vars = map,
                Err(e) => errors.push(format!("definition {slug} env_vars deserialize: {e}")),
            },
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
    }
    errors
}

/// Strip and persist a definition record's env_vars.
///
/// Mirrors the empty-projection guard in
/// [`strip_and_persist_agent_secrets_with`]: when the definition is
/// `secrets_unavailable` (its `env_vars_ref` failed to hydrate on load,
/// leaving `env_vars` empty in memory), preserve the existing ref on the
/// empty-projection branch rather than orphaning the live generation.
pub(crate) fn strip_and_persist_definition_secrets_with<S: ProjectionStore>(
    store: &S,
    record: &mut ManagedAgentRecord,
) {
    let unavailable = record.secrets_unavailable;
    let Some(slug) = record.slug.as_deref().map(str::to_string) else {
        return;
    };
    let inline_env = if !record.env_vars.is_empty() {
        serialize_env_map(&record.env_vars).ok()
    } else {
        None
    };
    let env_key = |gen: &str| definition_env_key(&slug, gen);
    let env_ctx = format!("definition:{slug} env_vars");
    let fields = [FieldSave {
        coord_key_fn: &env_key,
        value: inline_env.as_deref(),
        existing_ref: record.env_vars_ref.as_deref(),
        context: &env_ctx,
    }];
    let outcomes = write_secrets_batched(store, &fields);
    match &outcomes[0] {
        WriteOutcome::Persisted { gen } => {
            record.env_vars.clear();
            record.env_vars_ref = Some(gen.clone());
        }
        WriteOutcome::Nothing if unavailable => {}
        WriteOutcome::KeptInline { .. } | WriteOutcome::Nothing => {
            record.env_vars_ref = None;
        }
    }
}

/// Hydrate all secret fields in `records`, returning pubkeys of unavailable agents.
pub(crate) fn hydrate_all_secrets_for_records<S: ProjectionStore>(
    store: &S,
    records: &mut [ManagedAgentRecord],
) -> Vec<String> {
    let mut unavailable = Vec::new();
    for record in records.iter_mut() {
        if record.pubkey.is_empty() {
            let errors = hydrate_definition_secrets_with(store, record);
            for e in &errors {
                eprintln!("buzz-desktop: {e}");
            }
            // A definition whose env_vars_ref could not be hydrated is now
            // holding an EMPTY env map with a live ref. Mark it unavailable so
            // the strip-on-save path preserves the ref instead of committing
            // the empty projection (which would orphan the still-live
            // generation permanently). Definitions carry a slug, not a pubkey,
            // so they are not pushed to the pubkey-keyed `unavailable` summary.
            if !errors.is_empty() {
                record.secrets_unavailable = true;
            }
        } else {
            let errors = hydrate_agent_secrets_with(store, record);
            for e in &errors {
                eprintln!("buzz-desktop: {e}");
            }
            if !errors.is_empty() {
                record.secrets_unavailable = true;
                unavailable.push(record.pubkey.clone());
            }
        }
    }
    unavailable
}

/// Strip and persist all secret fields in `records`.
pub(crate) fn strip_and_persist_all_for_records<S: ProjectionStore>(
    store: &S,
    records: &mut [ManagedAgentRecord],
) {
    for record in records.iter_mut() {
        if record.pubkey.is_empty() {
            strip_and_persist_definition_secrets_with(store, record);
        } else {
            strip_and_persist_agent_secrets_with(store, record);
        }
    }
}

// ── Boot-migration transition (W1-safe) ───────────────────────────────────
//
// The ordinary save path (`strip_and_persist_*`) treats an empty inline field
// on an AVAILABLE record as a deliberate user-clear and drops the ref. The
// boot migration must NOT: it re-reads ALREADY-PROJECTED records straight off
// disk (empty inline + live ref, and `secrets_unavailable` is `#[serde(skip)]`
// so always `false`), which the save path would read as a clear and wipe every
// committed ref on the second launch. This transition is the migration's own
// contract: project only a non-empty inline value; preserve an existing ref
// when inline is absent; never infer a clear from a raw projected record.

/// Outcome of one field's boot-migration transition.
///
/// The caller applies the record mutation implied by each arm — the transition
/// itself is coordinate-generic (it does not know which record field it serves)
/// so it can back the agent tiers here AND the custom-harness surface, which
/// carries a single `env` map under its own keyring namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FieldMigration {
    /// Inline value written and read-back verified under a new generation.
    /// Caller: clear the inline field and set its `*_ref` to `gen`.
    Projected { gen: String },
    /// Inline absent, an existing ref is present. Caller: leave the ref
    /// untouched — the committed generation stays live.
    Preserved,
    /// No projection: inline absent with no existing ref, OR the keyring write
    /// failed (value kept inline for retry). Caller: leave the field as-is; the
    /// migration never sets a ref it did not write.
    Cleared,
}

/// Field-granular boot-migration transition. Projects `inline` into the keyring
/// under a fresh generation when present; otherwise preserves `existing_ref`
/// without ever clearing it.
///
/// Distinct from the strip-on-save seam: an absent inline value here means
/// "already projected on a prior launch," never "the user cleared it," so a
/// present ref is preserved rather than dropped. This is the seam Hayt's
/// custom-harness boot migration consumes so the two surfaces share one
/// W1-safe semantic instead of forking it.
pub(crate) fn migrate_inline_field<S: ProjectionStore>(
    store: &S,
    coord_key_fn: impl Fn(&str) -> String,
    inline: Option<&str>,
    existing_ref: Option<&str>,
    context: &str,
) -> FieldMigration {
    match write_secret(store, &coord_key_fn, inline, context) {
        WriteOutcome::Persisted { gen } => {
            cancel_gc_candidacy(store, &coord_key_fn(&gen));
            FieldMigration::Projected { gen }
        }
        // Inline absent: `write_secret` attempted nothing. Preserve an existing
        // ref (already-projected record) instead of treating empty as a clear.
        WriteOutcome::Nothing if existing_ref.is_some() => FieldMigration::Preserved,
        WriteOutcome::Nothing | WriteOutcome::KeptInline { .. } => FieldMigration::Cleared,
    }
}

/// Boot-migrate one instance record's secret fields at field granularity.
/// W1-safe: an already-projected record (empty inline, live refs) is left with
/// its refs intact rather than having them cleared.
fn migrate_agent_secrets_with<S: ProjectionStore>(store: &S, record: &mut ManagedAgentRecord) {
    let pubkey = record.pubkey.clone();
    let inline_env = if !record.env_vars.is_empty() {
        serialize_env_map(&record.env_vars).ok()
    } else {
        None
    };
    if let FieldMigration::Projected { gen } = migrate_inline_field(
        store,
        |gen| agent_env_key(&pubkey, gen),
        inline_env.as_deref(),
        record.env_vars_ref.as_deref(),
        &format!("agent:{pubkey} env_vars"),
    ) {
        record.env_vars.clear();
        record.env_vars_ref = Some(gen);
    }

    let auth_val = record.auth_tag.clone();
    if let FieldMigration::Projected { gen } = migrate_inline_field(
        store,
        |gen| agent_auth_tag_key(&pubkey, gen),
        auth_val.as_deref(),
        record.auth_tag_ref.as_deref(),
        &format!("agent:{pubkey} auth_tag"),
    ) {
        record.auth_tag = None;
        record.auth_tag_ref = Some(gen);
    }

    if let BackendKind::Provider {
        ref id,
        ref mut config,
    } = record.backend
    {
        let serialized = if !config.is_null() {
            serialize_provider_config(config).ok()
        } else {
            None
        };
        if let FieldMigration::Projected { gen } = migrate_inline_field(
            store,
            |gen| agent_provider_config_key(&pubkey, gen),
            serialized.as_deref(),
            record.provider_config_ref.as_deref(),
            &format!("agent:{pubkey} provider_config ({id})"),
        ) {
            *config = serde_json::Value::Null;
            record.provider_config_ref = Some(gen);
        }
    }
}

/// Boot-migrate one definition record's `env_vars` at field granularity.
/// The definition-tier mirror of [`migrate_agent_secrets_with`].
fn migrate_definition_secrets_with<S: ProjectionStore>(store: &S, record: &mut ManagedAgentRecord) {
    let Some(slug) = record.slug.as_deref().map(str::to_string) else {
        return;
    };
    let inline_env = if !record.env_vars.is_empty() {
        serialize_env_map(&record.env_vars).ok()
    } else {
        None
    };
    if let FieldMigration::Projected { gen } = migrate_inline_field(
        store,
        |gen| definition_env_key(&slug, gen),
        inline_env.as_deref(),
        record.env_vars_ref.as_deref(),
        &format!("definition:{slug} env_vars"),
    ) {
        record.env_vars.clear();
        record.env_vars_ref = Some(gen);
    }
}

/// Boot-migrate all secret fields in `records` at field granularity. Returns
/// `true` when any record's ref set changed (the caller then rewrites JSON).
pub(crate) fn migrate_all_secrets_for_records<S: ProjectionStore>(
    store: &S,
    records: &mut [ManagedAgentRecord],
) -> bool {
    let mut changed = false;
    for record in records.iter_mut() {
        let before = (
            record.env_vars_ref.clone(),
            record.auth_tag_ref.clone(),
            record.provider_config_ref.clone(),
        );
        if record.pubkey.is_empty() {
            migrate_definition_secrets_with(store, record);
        } else {
            migrate_agent_secrets_with(store, record);
        }
        if before
            != (
                record.env_vars_ref.clone(),
                record.auth_tag_ref.clone(),
                record.provider_config_ref.clone(),
            )
        {
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
#[path = "secret_seam_tests.rs"]
mod tests;
