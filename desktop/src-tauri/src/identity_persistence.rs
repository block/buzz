//! Three-valued imported-identity persistence outcome (P27-C1).
//!
//! The base persistence primitive
//! [`persist_imported_identity`](crate::app_state::persist_imported_identity)
//! returns a binary `Result`: `Ok(storage)` on success, `Err` otherwise. That
//! shape is unsafe for the identity-transition coordinator, because the kernel
//! is NOT transactional — a returned `Err` does NOT prove no durable key write
//! happened. `persist_identity_to_keyring` calls `store.store(...)` FIRST and
//! can still return `Err` after that durable mutation succeeded (read-back
//! verify failure, or marker creation failing with the emergency file write
//! also failing). A coordinator that compensates the drained runtimes on any
//! `Err` can leave durable identity B on disk beside live in-memory identity A
//! — and a later reachable-keyring restart activates B (the P25/P26
//! split-state class, now originating INSIDE the persistence call).
//!
//! This module wraps the proven kernel and classifies its result against
//! DURABLE FACT — never the helper's `Ok`/`Err` — into three outcomes:
//!
//! - [`PersistenceOutcome::DefinitelyUnchanged`] — proven: no durable B written
//!   / durable A still canonical. The ONLY outcome the coordinator may
//!   compensate.
//! - [`PersistenceOutcome::Committed`] — durable B proven canonical. The
//!   coordinator MUST finish the in-memory B commit + readiness/scope clear;
//!   compensation is forbidden. Marker/file-cleanup degradation is reported
//!   without unwinding the swap.
//! - [`PersistenceOutcome::Indeterminate`] — neither proven. The coordinator
//!   latches the durable fail-closed state (P28-C1): no compensation, no scope
//!   clear, drained runtimes stay down, and the process must not treat either
//!   identity as committed until a later reconciliation or restart proves one
//!   durable state.
//!
//! On a kernel `Err`, [`persist_imported_identity_classified`] performs
//! read-after-error reconciliation UNDER the caller's still-held transition
//! guards: it re-reads the durable stores (keyring read-back + `identity.key`)
//! and classifies from what it finds. A backend unreachable on re-read yields
//! `Indeterminate` — the fail-closed default.

use crate::app_state::{load_key_file, IdentityKeyStore, IdentityStorage, IDENTITY_KEY_NAME};
use nostr::{Keys, ToBech32};

/// The durable outcome of an imported-identity persistence attempt, classified
/// from durable fact rather than the kernel's `Ok`/`Err` (P27-C1).
#[derive(Debug)]
pub(crate) enum PersistenceOutcome {
    /// No durable B was written; durable A is still canonical. The only outcome
    /// the coordinator may compensate.
    DefinitelyUnchanged,
    /// Durable B is proven canonical. The coordinator must finish the B commit;
    /// compensation is forbidden. `storage` records where B durably landed.
    Committed(IdentityStorage),
    /// Neither state is proven. The coordinator latches the durable fail-closed
    /// state (P28-C1). `reason` is a diagnostic, never a control signal.
    Indeterminate(String),
}

/// Persist an imported identity and classify the result against durable fact.
///
/// `persist` runs the proven kernel
/// ([`persist_imported_identity`](crate::app_state::persist_imported_identity))
/// which stores B, verifies the read-back, writes the marker, and deletes the
/// legacy file — falling back to the `0o600` file when the keyring is
/// unavailable. On `Ok`, B is durably proven and the outcome is `Committed`.
/// On `Err`, the kernel may have durably written B before failing, so this
/// re-reads the durable stores under the caller's held guards and classifies:
///
/// - keyring read-back matches B, or `identity.key` holds B ⇒ `Committed`;
/// - keyring reachable-and-not-B AND `identity.key` absent-or-A ⇒
///   `DefinitelyUnchanged` (B never landed, A intact);
/// - anything still ambiguous (keyring unreachable on re-read) ⇒
///   `Indeterminate` (fail closed).
///
/// `expected` is the identity being imported (B). `store` and `legacy_path`
/// name the durable backends to re-read.
pub(crate) fn persist_imported_identity_classified(
    keys: &Keys,
    store: &impl IdentityKeyStore,
    legacy_path: &std::path::Path,
    persist: impl FnOnce() -> Result<IdentityStorage, String>,
) -> PersistenceOutcome {
    match persist() {
        Ok(storage) => PersistenceOutcome::Committed(storage),
        Err(kernel_err) => reconcile_after_error(keys, store, legacy_path, &kernel_err),
    }
}

/// Read-after-error reconciliation (P27-C1): the kernel returned `Err`, which
/// does not prove B was not durably written. Re-read the durable stores and
/// classify from what they hold. Runs under the caller's held transition
/// guards, so no concurrent transition can move the durable state mid-read.
fn reconcile_after_error(
    expected: &Keys,
    store: &impl IdentityKeyStore,
    legacy_path: &std::path::Path,
    kernel_err: &str,
) -> PersistenceOutcome {
    let expected_nsec = match expected.secret_key().to_bech32() {
        Ok(nsec) => nsec,
        // Cannot encode the target for comparison — cannot prove either state.
        Err(e) => {
            return PersistenceOutcome::Indeterminate(format!(
                "persist failed ({kernel_err}); could not encode target key for \
                 read-after-error reconciliation: {e}"
            ));
        }
    };
    let expected_pubkey = expected.public_key();

    // (1) Durable file: an `identity.key` holding B proves B landed durably —
    // the kernel's file fallback (or emergency write) succeeded before failing
    // on a later best-effort step. A file holding A (or no file) means B did
    // not reach the file backend.
    let file_holds_b = match legacy_path.exists() {
        false => false,
        true => match load_key_file(legacy_path) {
            Ok(file_keys) => file_keys.public_key() == expected_pubkey,
            // A corrupt file proves nothing about B's durability.
            Err(_) => {
                return PersistenceOutcome::Indeterminate(format!(
                    "persist failed ({kernel_err}); identity.key present but unreadable \
                     on read-after-error reconciliation"
                ));
            }
        },
    };
    if file_holds_b {
        return PersistenceOutcome::Committed(IdentityStorage::LocalFile);
    }

    // (2) Durable keyring: a read-back matching B proves B is canonical in the
    // keyring even though a later step (marker/file cleanup) failed.
    match store.verify_stored(IDENTITY_KEY_NAME, &expected_nsec) {
        Ok(true) => PersistenceOutcome::Committed(IdentityStorage::SystemKeyring),
        // Keyring reachable and does NOT hold B, and the file does not hold B
        // (checked above): B never landed durably, A is intact.
        Ok(false) => PersistenceOutcome::DefinitelyUnchanged,
        // Keyring unreachable on re-read: cannot prove either state. Fail
        // closed — the coordinator latches Indeterminate.
        Err(e) => PersistenceOutcome::Indeterminate(format!(
            "persist failed ({kernel_err}); durable state unprovable — keyring \
             unreachable on read-after-error reconciliation: {e}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret_store::KeyringProbe;
    use std::cell::RefCell;

    /// Minimal fake exercising only the durable re-reads the classifier makes.
    /// `keyring`/`file` model what each backend HOLDS after the kernel `Err`;
    /// `keyring_unreachable` makes the read-back re-read fail.
    struct ReReadStore {
        keyring: RefCell<Option<String>>,
        keyring_unreachable: bool,
    }

    impl ReReadStore {
        fn holding(nsec: Option<&str>) -> Self {
            Self {
                keyring: RefCell::new(nsec.map(str::to_string)),
                keyring_unreachable: false,
            }
        }
        fn unreachable() -> Self {
            Self {
                keyring: RefCell::new(None),
                keyring_unreachable: true,
            }
        }
    }

    impl IdentityKeyStore for ReReadStore {
        fn probe(&self, _name: &str) -> KeyringProbe {
            KeyringProbe::Present
        }
        fn load(&self, _name: &str) -> Result<Option<String>, String> {
            Ok(self.keyring.borrow().clone())
        }
        fn store(&self, _name: &str, value: &str) -> Result<(), String> {
            *self.keyring.borrow_mut() = Some(value.to_string());
            Ok(())
        }
        fn delete(&self, _name: &str) -> Result<(), String> {
            *self.keyring.borrow_mut() = None;
            Ok(())
        }
        fn verify_stored(&self, _name: &str, expected: &str) -> Result<bool, String> {
            if self.keyring_unreachable {
                return Err("keyring unreachable".to_string());
            }
            Ok(self.keyring.borrow().as_deref() == Some(expected))
        }
    }

    fn nsec_of(keys: &Keys) -> String {
        keys.secret_key().to_bech32().unwrap()
    }

    fn tmp_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("identity.key")
    }

    #[test]
    fn ok_kernel_is_committed() {
        let b = Keys::generate();
        let store = ReReadStore::holding(Some(&nsec_of(&b)));
        let dir = tempfile::tempdir().unwrap();
        let outcome = persist_imported_identity_classified(&b, &store, &tmp_file(&dir), || {
            Ok(IdentityStorage::SystemKeyring)
        });
        assert!(matches!(
            outcome,
            PersistenceOutcome::Committed(IdentityStorage::SystemKeyring)
        ));
    }

    /// P27-C1 fixture (a): store(B) success + verify error + file-fallback
    /// failure. The kernel returns `Err`, but the keyring durably holds B —
    /// read-after-error reconciliation must classify Committed, NOT compensate.
    #[test]
    fn err_but_keyring_holds_b_is_committed() {
        let b = Keys::generate();
        let store = ReReadStore::holding(Some(&nsec_of(&b)));
        let dir = tempfile::tempdir().unwrap();
        let outcome = persist_imported_identity_classified(&b, &store, &tmp_file(&dir), || {
            Err("verify error; file fallback failed".to_string())
        });
        assert!(
            matches!(
                outcome,
                PersistenceOutcome::Committed(IdentityStorage::SystemKeyring)
            ),
            "durable B in keyring must classify Committed even on kernel Err"
        );
    }

    /// P27-C1 fixture (b): store/verify success + marker failure + both file
    /// attempts failing. The emergency file write LANDED B before the outer
    /// fallback error, so `identity.key` holds B — reconciliation classifies
    /// Committed(LocalFile).
    #[test]
    fn err_but_file_holds_b_is_committed() {
        let b = Keys::generate();
        // Keyring does not hold B (marker/keyring path failed); the emergency
        // file write landed B.
        let store = ReReadStore::holding(None);
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_file(&dir);
        crate::app_state::save_key_file(&path, &b).unwrap();
        let outcome = persist_imported_identity_classified(&b, &store, &path, || {
            Err("marker + file attempts failed".to_string())
        });
        assert!(matches!(
            outcome,
            PersistenceOutcome::Committed(IdentityStorage::LocalFile)
        ));
    }

    /// Kernel `Err` with A intact everywhere: keyring reachable and NOT holding
    /// B, no B file. B never landed — the only outcome permitted to compensate.
    #[test]
    fn err_and_a_intact_is_definitely_unchanged() {
        let a = Keys::generate();
        let b = Keys::generate();
        // Keyring still holds A (B never stored); no leftover file.
        let store = ReReadStore::holding(Some(&nsec_of(&a)));
        let dir = tempfile::tempdir().unwrap();
        let outcome = persist_imported_identity_classified(&b, &store, &tmp_file(&dir), || {
            Err("store(B) never succeeded".to_string())
        });
        assert!(matches!(outcome, PersistenceOutcome::DefinitelyUnchanged));
    }

    /// Kernel `Err` with the keyring unreachable on re-read and no B file:
    /// neither state is provable — fail closed to Indeterminate.
    #[test]
    fn err_and_keyring_unreachable_is_indeterminate() {
        let b = Keys::generate();
        let store = ReReadStore::unreachable();
        let dir = tempfile::tempdir().unwrap();
        let outcome = persist_imported_identity_classified(&b, &store, &tmp_file(&dir), || {
            Err("store then verify unreachable".to_string())
        });
        assert!(
            matches!(outcome, PersistenceOutcome::Indeterminate(_)),
            "unprovable durable state must fail closed"
        );
    }
}
