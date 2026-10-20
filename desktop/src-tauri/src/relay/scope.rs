use super::relay_http_base_url;

/// Fail closed when a caller-captured relay scope no longer matches the
/// relay a command actually resolved.
///
/// Long-lived UI callbacks (e.g. the Projects agent submit flow) capture the
/// community relay before their first await; a workspace switch during that
/// await would otherwise retarget the eventual publication to the new
/// tenant's relay. Callers pass the captured scope as a ws(s) URL; it is
/// normalized through [`relay_http_base_url`] and compared against the base
/// the command resolved once and uses for every side effect. `None` preserves
/// the unscoped behavior for callers without a tenant boundary.
pub fn assert_expected_relay_scope(
    expected_relay_url: Option<&str>,
    resolved_api_base_url: &str,
) -> Result<(), String> {
    let Some(expected) = expected_relay_url.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let expected_base = relay_http_base_url(expected);
    if expected_base != resolved_api_base_url.trim().trim_end_matches('/') {
        return Err(
            "active community changed before the message was submitted; not sent".to_string(),
        );
    }
    Ok(())
}

/// A workspace-relay read that has passed the caller-captured scope check.
///
/// The only constructor is [`bind_expected_relay_scope`], so any side effect
/// that takes this type is proven — by construction — to consume the exact
/// value the check passed on, never a re-read of the mutable override. This
/// closes the check/use gap where a workspace switch landing between a scope
/// assertion and the side effect retargets it to a tenant the caller never
/// validated.
#[derive(Debug)]
pub struct ScopedWorkspaceRelay(String);

impl ScopedWorkspaceRelay {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validate a caller-captured relay scope against one workspace-relay read
/// and bind that exact read for the side effect to consume.
///
/// `None` preserves the unscoped behavior for callers without a tenant
/// boundary — the read is still bound so the side effect stays single-read.
pub fn bind_expected_relay_scope(
    expected_relay_url: Option<&str>,
    workspace_relay_url: String,
) -> Result<ScopedWorkspaceRelay, String> {
    assert_expected_relay_scope(
        expected_relay_url,
        &relay_http_base_url(&workspace_relay_url),
    )?;
    Ok(ScopedWorkspaceRelay(workspace_relay_url))
}

/// Fail closed when a caller-captured signer identity no longer matches the
/// identity a command actually read.
///
/// The relay URL and the signing keys live under separate locks and a
/// workspace switch mutates them in sequence, so a caller that only pins the
/// relay can still have its event signed — and its NIP-98 auth minted — by
/// the *new* tenant's identity if the switch lands between the URL check and
/// the key read. Callers capture the expected owner pubkey together with the
/// relay scope; commands read one identity snapshot, assert it here, and use
/// that exact snapshot for every signature. `None` preserves the unscoped
/// behavior for callers without a tenant boundary.
pub fn assert_expected_signer(
    expected_signer_pubkey: Option<&str>,
    actual_signer_hex: &str,
) -> Result<(), String> {
    let Some(expected) = expected_signer_pubkey
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    if !expected.eq_ignore_ascii_case(actual_signer_hex) {
        return Err(
            "active identity changed before the message was submitted; not sent".to_string(),
        );
    }
    Ok(())
}

/// A workspace-signer read that has passed the caller-captured identity check.
///
/// The only constructor is [`bind_expected_signer`], so side effects consume
/// the exact owner read that was validated rather than a stale pre-await value.
#[derive(Debug)]
pub struct ScopedWorkspaceSigner(String);

impl ScopedWorkspaceSigner {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validate a caller-captured signer against one active-owner read and bind
/// that exact read for the side effect to consume. `None` preserves unscoped
/// callers while still making the owner input single-read.
pub fn bind_expected_signer(
    expected_signer_pubkey: Option<&str>,
    actual_signer_hex: String,
) -> Result<ScopedWorkspaceSigner, String> {
    assert_expected_signer(expected_signer_pubkey, &actual_signer_hex)?;
    Ok(ScopedWorkspaceSigner(actual_signer_hex))
}

#[cfg(test)]
mod tests {
    use super::{
        assert_expected_relay_scope, assert_expected_signer, bind_expected_relay_scope,
        bind_expected_signer,
    };

    #[test]
    fn matching_scope_passes_across_ws_http_normalization() {
        assert_expected_relay_scope(Some("wss://tenant-a.example"), "https://tenant-a.example")
            .unwrap();
        assert_expected_relay_scope(Some("ws://localhost:3000"), "http://localhost:3000").unwrap();
        // Trailing-slash and whitespace tolerance mirrors relay_http_base_url.
        assert_expected_relay_scope(
            Some(" wss://tenant-a.example/ "),
            "https://tenant-a.example/",
        )
        .unwrap();
    }

    #[test]
    fn changed_scope_fails_closed() {
        let error =
            assert_expected_relay_scope(Some("wss://tenant-a.example"), "https://tenant-b.example")
                .unwrap_err();
        assert!(error.contains("active community changed"), "{error}");
    }

    #[test]
    fn absent_scope_preserves_unscoped_sends() {
        assert_expected_relay_scope(None, "https://anything.example").unwrap();
        assert_expected_relay_scope(Some(""), "https://anything.example").unwrap();
        assert_expected_relay_scope(Some("   "), "https://anything.example").unwrap();
    }

    #[test]
    fn bound_scope_is_immune_to_a_switch_landing_after_the_bind() {
        // Models the round-7 startup race: the caller captured tenant A, the
        // post-preflight bind reads the workspace relay while it is still A,
        // and THEN the switch to B lands — after the check, before the spawn.
        // The spawn consumes the BOUND value, not a re-read, so the pair can
        // only ever be keyed to the tenant the caller validated; the switch
        // mutates state the spawn no longer consults.
        let mut workspace = "wss://tenant-a.example".to_string();
        let bound =
            bind_expected_relay_scope(Some("wss://tenant-a.example"), workspace.clone()).unwrap();
        workspace = "wss://tenant-b.example".to_string(); // the switch lands post-check
        assert_eq!(bound.as_str(), "wss://tenant-a.example");
        assert_ne!(
            bound.as_str(),
            workspace,
            "spawn input must be the checked value"
        );
    }

    #[test]
    fn bind_fails_closed_when_the_switch_lands_before_the_read() {
        // The switch landed during the preflight await, so the one workspace
        // read already sees tenant B: no relay may be released to the spawn.
        let error = bind_expected_relay_scope(
            Some("wss://tenant-a.example"),
            "wss://tenant-b.example".to_string(),
        )
        .unwrap_err();
        assert!(error.contains("active community changed"), "{error}");
    }

    #[test]
    fn bind_returns_the_exact_read_for_unscoped_callers() {
        let bound = bind_expected_relay_scope(None, "wss://anything.example".to_string()).unwrap();
        assert_eq!(bound.as_str(), "wss://anything.example");
    }

    // The round-7 pair-key regression moved to
    // `managed_agents::runtime::tests::production_spawn_key_derives_from_the_bound_relay_not_the_post_switch_workspace`,
    // which exercises `bound_runtime_key` — the seam production spawn keys on —
    // instead of reconstructing the derivation by hand here.

    #[test]
    fn matching_signer_passes_case_insensitively() {
        let keys = nostr::Keys::generate();
        let hex = keys.public_key().to_hex();
        assert_expected_signer(Some(&hex), &hex).unwrap();
        assert_expected_signer(Some(&hex.to_ascii_uppercase()), &hex).unwrap();
        assert_expected_signer(Some(&format!("  {hex} ")), &hex).unwrap();
    }

    #[test]
    fn changed_signer_fails_closed() {
        // Models the workspace-switch race: the caller captured tenant A's
        // owner identity, but the switch landed before the command read the
        // keys, so the snapshot now holds tenant B's identity.
        let captured = nostr::Keys::generate().public_key().to_hex();
        let switched = nostr::Keys::generate().public_key().to_hex();
        let error = assert_expected_signer(Some(&captured), &switched).unwrap_err();
        assert!(error.contains("active identity changed"), "{error}");
    }

    #[test]
    fn signer_bind_fails_closed_after_same_relay_identity_switch() {
        let captured = nostr::Keys::generate().public_key().to_hex();
        let switched = nostr::Keys::generate().public_key().to_hex();
        let error = bind_expected_signer(Some(&captured), switched).unwrap_err();
        assert!(error.contains("active identity changed"), "{error}");
    }

    #[test]
    fn signer_bind_returns_exact_read_for_scoped_and_unscoped_callers() {
        let actual = nostr::Keys::generate().public_key().to_hex();
        let scoped = bind_expected_signer(Some(&actual), actual.clone()).unwrap();
        assert_eq!(scoped.as_str(), actual);

        let unscoped_actual = nostr::Keys::generate().public_key().to_hex();
        let unscoped = bind_expected_signer(None, unscoped_actual.clone()).unwrap();
        assert_eq!(unscoped.as_str(), unscoped_actual);
    }

    #[test]
    fn absent_signer_preserves_unscoped_sends() {
        let hex = nostr::Keys::generate().public_key().to_hex();
        assert_expected_signer(None, &hex).unwrap();
        assert_expected_signer(Some(""), &hex).unwrap();
        assert_expected_signer(Some("   "), &hex).unwrap();
    }
}
