//! Kind translation between standard Nostr kinds and Buzz custom kinds.
//!
//! # ⚠️ Architectural limitation
//!
//! Translating a Nostr event's `kind` field **invalidates its signature**. The
//! Nostr event ID is `SHA-256([0, pubkey, created_at, kind, tags, content])`, so
//! any kind mutation produces a different ID and a broken Schnorr signature.
//!
//! This translator is intentionally designed for **Buzz-internal use only**,
//! where events are re-signed by the proxy's shadow keypair after translation.
//! It must never be used in a standard Nostr interop path where signature
//! verification is expected to pass.

use buzz_core::kind::{
    KIND_DM_CREATED, KIND_NIP29_DELETE_EVENT, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_EDIT,
    KIND_STREAM_MESSAGE_V2,
};

/// Translates Nostr event kinds between standard and Buzz-internal values.
pub struct KindTranslator;

impl KindTranslator {
    /// Create a new [`KindTranslator`].
    pub fn new() -> Self {
        Self
    }

    /// Translate a standard Nostr kind to the equivalent Buzz kind.
    /// Unknown kinds pass through unchanged.
    ///
    /// # ⚠️ Lossy mapping — round-tripping is NOT lossless
    ///
    /// Multiple standard Nostr kinds collapse onto the same Buzz kind.
    /// This is intentional: Buzz's internal kind space is smaller than the
    /// full Nostr kind space, and the proxy re-signs events anyway (see module
    /// doc), so the original kind is not preserved.
    ///
    /// **Do not use `to_standard(to_buzz(k))` expecting to recover `k`.**
    /// The round-trip is only lossless for kinds that have a 1-to-1 mapping.
    ///
    /// | Standard kind(s)       | Buzz kind               | Lossy? |
    /// |------------------------|---------------------------|--------|
    /// | 1, 40, 42              | `KIND_STREAM_MESSAGE`     | ✅ yes |
    /// | 41, 44                 | `KIND_STREAM_MESSAGE_EDIT`| ✅ yes |
    /// | 4                      | `KIND_DM_CREATED`         | no     |
    /// | 43                     | `KIND_NIP29_DELETE_EVENT` | no     |
    /// | anything else          | unchanged (pass-through)  | no     |
    pub fn to_buzz(&self, standard_kind: u32) -> u32 {
        match standard_kind {
            1 => KIND_STREAM_MESSAGE,
            4 => KIND_DM_CREATED,
            40 => KIND_STREAM_MESSAGE,
            41 => KIND_STREAM_MESSAGE_EDIT,
            42 => KIND_STREAM_MESSAGE,
            43 => KIND_NIP29_DELETE_EVENT,
            44 => KIND_STREAM_MESSAGE_EDIT,
            k => k,
        }
    }

    /// Translate a Buzz kind back to the canonical standard Nostr kind.
    /// Unknown kinds pass through unchanged.
    ///
    /// Returns the **canonical** standard kind for each Buzz kind. Because
    /// `to_buzz` is lossy (multiple standard kinds map to one Buzz kind),
    /// this function always returns the primary/canonical standard kind — it
    /// cannot recover the original kind if it was one of the secondary mappings.
    ///
    /// For example: `to_standard(KIND_STREAM_MESSAGE)` returns `42` (NIP-28
    /// channel message), not `1` or `40`, even if the event was originally
    /// kind 1 or 40.
    ///
    /// | Buzz kind                | Standard kind | Notes                     |
    /// |----------------------------|---------------|---------------------------|
    /// | `KIND_STREAM_MESSAGE`      | 42            | NIP-28 channel message    |
    /// | `KIND_STREAM_MESSAGE_V2`   | 42            | Rich format → plain 42    |
    /// | `KIND_STREAM_MESSAGE_EDIT` | 41            | NIP-28 channel message edit |
    /// | `KIND_DM_CREATED`          | 4             | Encrypted DM              |
    /// | `KIND_NIP29_DELETE_EVENT`  | 43            | NIP-29 delete             |
    /// | anything else              | unchanged     | pass-through              |
    pub fn to_standard(&self, buzz_kind: u32) -> u32 {
        match buzz_kind {
            k if k == KIND_STREAM_MESSAGE => 42, // NIP-28 channel message (was 1)
            k if k == KIND_STREAM_MESSAGE_V2 => 42, // Rich format → plain kind:42
            k if k == KIND_STREAM_MESSAGE_EDIT => 41,
            k if k == KIND_DM_CREATED => 4,
            k if k == KIND_NIP29_DELETE_EVENT => 43,
            k => k,
        }
    }

    /// Returns `true` if `kind` has a non-identity mapping in either direction.
    pub fn is_translatable(&self, kind: u32) -> bool {
        self.to_buzz(kind) != kind || self.to_standard(kind) != kind
    }
}

impl Default for KindTranslator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::kind::{
        KIND_DM_CREATED, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_EDIT, KIND_STREAM_MESSAGE_V2,
    };

    #[test]
    fn standard_to_buzz() {
        let t = KindTranslator::new();
        assert_eq!(t.to_buzz(1), KIND_STREAM_MESSAGE);
        assert_eq!(t.to_buzz(4), KIND_DM_CREATED);
        assert_eq!(t.to_buzz(40), KIND_STREAM_MESSAGE);
        assert_eq!(t.to_buzz(41), KIND_STREAM_MESSAGE_EDIT);
    }

    #[test]
    fn buzz_to_standard() {
        let t = KindTranslator::new();
        assert_eq!(t.to_standard(KIND_STREAM_MESSAGE), 42);
        assert_eq!(t.to_standard(KIND_STREAM_MESSAGE_V2), 42);
        assert_eq!(t.to_standard(KIND_STREAM_MESSAGE_EDIT), 41);
        assert_eq!(t.to_standard(KIND_DM_CREATED), 4);
    }

    #[test]
    fn stream_message_v2_round_trip() {
        let t = KindTranslator::new();
        // kind:42 → KIND_STREAM_MESSAGE (lossy collapse), then back → 42
        assert_eq!(t.to_standard(t.to_buzz(42)), 42);
    }

    #[test]
    fn unknown_kinds_pass_through() {
        let t = KindTranslator::new();
        assert_eq!(t.to_buzz(9999), 9999);
        assert_eq!(t.to_buzz(0), 0);
        assert_eq!(t.to_standard(12345), 12345);
        assert_eq!(t.to_standard(0), 0);
    }

    #[test]
    fn is_translatable() {
        let t = KindTranslator::new();
        assert!(t.is_translatable(1));
        assert!(t.is_translatable(4));
        assert!(t.is_translatable(KIND_STREAM_MESSAGE));
        assert!(t.is_translatable(KIND_STREAM_MESSAGE_V2));
        assert!(!t.is_translatable(9999));
        assert!(!t.is_translatable(0));
    }
}
