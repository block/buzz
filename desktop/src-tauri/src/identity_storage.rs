use nostr::Keys;

use crate::app_state::AppState;

/// Durable location of the active human identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum IdentityStorage {
    Ephemeral = 0,
    SystemKeyring = 1,
    LocalFile = 2,
    Environment = 3,
    /// No local user identity exists in a remote build.
    Absent = 4,
}

impl IdentityStorage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ephemeral => "ephemeral",
            Self::SystemKeyring => "system-keyring",
            Self::LocalFile => "local-file",
            Self::Environment => "environment",
            Self::Absent => "absent",
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::SystemKeyring,
            2 => Self::LocalFile,
            3 => Self::Environment,
            4 => Self::Absent,
            _ => Self::Ephemeral,
        }
    }
}

impl AppState {
    pub(crate) fn identity_storage(&self) -> IdentityStorage {
        IdentityStorage::from_u8(
            self.identity_storage
                .load(std::sync::atomic::Ordering::Acquire),
        )
    }

    pub(crate) fn set_identity_storage(&self, storage: IdentityStorage) {
        self.identity_storage
            .store(storage as u8, std::sync::atomic::Ordering::Release);
    }
}

/// Recovery state produced by identity resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryState {
    None,
    Lost,
    KeyringLocked,
}

/// Identity and persistence metadata produced by startup resolution.
pub(crate) struct ResolvedIdentity {
    pub(crate) keys: Keys,
    pub(crate) recovery: RecoveryState,
    pub(crate) storage: IdentityStorage,
}

/// Public-only snapshot read while the local key lock is held.
pub(crate) struct LocalIdentitySnapshot {
    pub(crate) pubkey: nostr::PublicKey,
    pub(crate) storage: IdentityStorage,
    pub(crate) lost: bool,
    pub(crate) locked: bool,
    pub(crate) reset_failed: bool,
}
