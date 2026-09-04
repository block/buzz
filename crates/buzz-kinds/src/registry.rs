//! The kind registry, assembled once at startup.
//!
//! [`register_builtin`] transcribes the relay's `required_scope_for_kind` /
//! `is_global_only_kind` / `requires_h_channel_scope` matrix (as of the kind
//! set in `buzz-core::kind` at the time this PR was written) into
//! [`KindDescriptor`] entries. A registry *miss* is the fail-closed reject
//! that today's `_ => Err("restricted: unknown event kind")` arm provides.
//!
//! Deliberately NOT covered here (see the PR description for why):
//! - `buzz_core::kind::is_relay_only_kind` — still runs before the registry
//!   lookup in `ingest_event_inner`, so relay-only kinds are never registered.
//! - `super::push_lease::KIND_PUSH_LEASE` (30350) — the relay's dedicated
//!   push-lease handling keeps its own inline special case for now.
//! - Read-gate (`P_GATED_KINDS` / `AUTHOR_ONLY_KINDS` / `RESULT_GATED_KINDS`)
//!   and command/moderation-command routing stay on their existing paths;
//!   `ReadGate` is declared on the descriptor for completeness but unused.

use std::collections::HashMap;

use buzz_auth::Scope;
use buzz_core::kind::*;

use crate::descriptor::{Authorship, KindDescriptor, ReadGate, RequiredScope, Scoping};
use crate::extension::KindExtension;

/// A lookup table from event kind to its [`KindDescriptor`].
#[derive(Default)]
pub struct KindRegistry {
    by_kind: HashMap<u32, KindDescriptor>,
}

impl KindRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a kind's descriptor.
    ///
    /// Panics on a duplicate kind: registration happens once at startup, and
    /// a duplicate is a build-time configuration error that must fail fast.
    pub fn register(&mut self, descriptor: KindDescriptor) {
        let kind = descriptor.kind;
        assert!(
            self.by_kind.insert(kind, descriptor).is_none(),
            "duplicate kind descriptor registered for kind {kind}"
        );
    }

    /// Look up a kind's descriptor. `None` means the kind is unregistered —
    /// callers must fail closed on a miss.
    pub fn get(&self, kind: u32) -> Option<&KindDescriptor> {
        self.by_kind.get(&kind)
    }

    /// Iterate every registered descriptor (used by tests to check parity
    /// against `buzz-core`'s kind matrix).
    pub fn iter(&self) -> impl Iterator<Item = &KindDescriptor> {
        self.by_kind.values()
    }
}

/// Build a `ClientWritable`, extension-less descriptor with a static scope.
fn static_kind(kind: u32, name: &'static str, scope: Scope, scoping: Scoping) -> KindDescriptor {
    KindDescriptor {
        kind,
        name,
        required_scope: RequiredScope::Static(scope),
        scoping,
        read_gate: ReadGate::Public,
        authorship: Authorship::ClientWritable,
        extension: None,
    }
}

/// Build a `ClientWritable`, extension-less descriptor with a static scope and
/// an explicit, non-`Public` read gate.
fn static_kind_gated(
    kind: u32,
    name: &'static str,
    scope: Scope,
    scoping: Scoping,
    read_gate: ReadGate,
) -> KindDescriptor {
    KindDescriptor {
        kind,
        name,
        required_scope: RequiredScope::Static(scope),
        scoping,
        read_gate,
        authorship: Authorship::ClientWritable,
        extension: None,
    }
}

/// NIP-29 `kind:9002` (edit-metadata) scope resolver: an `archived` tag
/// requires `Scope::AdminChannels`; otherwise `Scope::ChannelsWrite`.
///
/// Transcribed verbatim from the former inline match arm in
/// `handlers::ingest::required_scope_for_kind`.
struct EditMetadataExt;

impl KindExtension for EditMetadataExt {
    fn required_scope(&self, event: &nostr::Event) -> Scope {
        let has_archived = event
            .tags
            .iter()
            .any(|t| t.kind().to_string() == "archived");
        if has_archived {
            Scope::AdminChannels
        } else {
            Scope::ChannelsWrite
        }
    }
}

static EDIT_METADATA_EXT: EditMetadataExt = EditMetadataExt;

/// Register every built-in kind's declarative policy.
pub fn register_builtin(registry: &mut KindRegistry) {
    use Scoping::{ChannelOptional, ChannelRequired, Global};

    // --- User-owned global profile/content kinds --------------------------
    registry.register(static_kind(
        KIND_PROFILE,
        "profile",
        Scope::UsersWrite,
        Global,
    ));
    registry.register(static_kind(
        KIND_TEXT_NOTE,
        "text_note",
        Scope::MessagesWrite,
        Global,
    ));
    registry.register(static_kind(
        KIND_LONG_FORM,
        "long_form",
        Scope::MessagesWrite,
        Global,
    ));
    registry.register(static_kind(
        KIND_CONTACT_LIST,
        "contact_list",
        Scope::UsersWrite,
        Global,
    ));
    registry.register(static_kind(
        KIND_READ_STATE,
        "read_state",
        Scope::UsersWrite,
        Global,
    ));
    registry.register(static_kind(
        KIND_USER_STATUS,
        "user_status",
        Scope::UsersWrite,
        Global,
    ));
    registry.register(static_kind_gated(
        KIND_AGENT_ENGRAM,
        "agent_engram",
        Scope::UsersWrite,
        Global,
        ReadGate::Public,
    ));
    registry.register(static_kind_gated(
        KIND_EVENT_REMINDER,
        "event_reminder",
        Scope::UsersWrite,
        Global,
        // buzz_core::kind::AUTHOR_ONLY_KINDS.
        ReadGate::AuthorOnly,
    ));
    registry.register(static_kind(
        KIND_PERSONA,
        "persona",
        Scope::UsersWrite,
        Global,
    ));
    registry.register(static_kind(KIND_TEAM, "team", Scope::UsersWrite, Global));
    registry.register(static_kind(
        KIND_MANAGED_AGENT,
        "managed_agent",
        Scope::UsersWrite,
        Global,
    ));
    registry.register(static_kind_gated(
        KIND_PRIVATE_MANAGED_AGENT,
        "private_managed_agent",
        Scope::UsersWrite,
        Global,
        // buzz_core::kind::AUTHOR_ONLY_KINDS.
        ReadGate::AuthorOnly,
    ));
    registry.register(static_kind(
        KIND_TEAM_CATALOG,
        "team_catalog",
        Scope::UsersWrite,
        Global,
    ));

    // --- NIP-AM: agent turn metrics -----------------------------------------
    registry.register(static_kind_gated(
        KIND_AGENT_TURN_METRIC,
        "agent_turn_metric",
        Scope::MessagesWrite,
        Global,
        // In both `P_GATED_KINDS` and `RESULT_GATED_KINDS` upstream;
        // `ResultGated` is the strictly stronger of the two.
        ReadGate::ResultGated,
    ));

    // --- NIP-56 reports + product feedback (mod-queue only, never fanned out)
    registry.register(static_kind(
        KIND_REPORT,
        "report",
        Scope::MessagesWrite,
        ChannelOptional,
    ));
    registry.register(static_kind(
        KIND_PRODUCT_FEEDBACK,
        "product_feedback",
        Scope::MessagesWrite,
        ChannelOptional,
    ));

    // --- Community moderation commands (9040-9044) -------------------------
    // Direct, mod-authz-gated writes; scope only proves the transport can
    // submit message writes — the command handler owns the real
    // role/capability authorization. Never channel-scoped by a stray `h`.
    for (kind, name) in [
        (KIND_MODERATION_BAN, "moderation_ban"),
        (KIND_MODERATION_UNBAN, "moderation_unban"),
        (KIND_MODERATION_TIMEOUT, "moderation_timeout"),
        (KIND_MODERATION_UNTIMEOUT, "moderation_untimeout"),
        (KIND_MODERATION_RESOLVE_REPORT, "moderation_resolve_report"),
    ] {
        registry.register(static_kind(kind, name, Scope::MessagesWrite, Global));
    }

    // --- NIP-51 standard lists/sets, NIP-65 relay list, NIP-30 emoji -------
    for (kind, name) in [
        (KIND_MUTE_LIST, "mute_list"),
        (KIND_PIN_LIST, "pin_list"),
        (KIND_NIP65_RELAY_LIST_METADATA, "nip65_relay_list_metadata"),
        (KIND_BOOKMARK_LIST, "bookmark_list"),
        (KIND_FOLLOW_SET, "follow_set"),
        (KIND_BOOKMARK_SET, "bookmark_set"),
        (KIND_EMOJI_SET, "emoji_set"),
        (KIND_EMOJI_LIST, "emoji_list"),
        (KIND_AGENT_PROFILE, "agent_profile"),
    ] {
        registry.register(static_kind(kind, name, Scope::UsersWrite, Global));
    }

    // --- Channel-optional message-scoped kinds ------------------------------
    registry.register(static_kind(
        KIND_DELETION,
        "deletion",
        Scope::MessagesWrite,
        ChannelOptional,
    ));
    registry.register(static_kind(
        KIND_REACTION,
        "reaction",
        Scope::MessagesWrite,
        ChannelOptional,
    ));
    registry.register(static_kind_gated(
        KIND_GIFT_WRAP,
        "gift_wrap",
        Scope::MessagesWrite,
        ChannelOptional,
        // buzz_core::kind::P_GATED_KINDS.
        ReadGate::PGated,
    ));
    registry.register(static_kind(
        KIND_NIP29_DELETE_EVENT,
        "nip29_delete_event",
        Scope::MessagesWrite,
        ChannelRequired,
    ));

    // --- Channel-required stream/forum message kinds ------------------------
    for (kind, name) in [
        (KIND_STREAM_MESSAGE, "stream_message"),
        (KIND_STREAM_MESSAGE_V2, "stream_message_v2"),
        (KIND_STREAM_MESSAGE_EDIT, "stream_message_edit"),
        (KIND_STREAM_MESSAGE_PINNED, "stream_message_pinned"),
        (KIND_STREAM_MESSAGE_BOOKMARKED, "stream_message_bookmarked"),
        (KIND_STREAM_MESSAGE_SCHEDULED, "stream_message_scheduled"),
        (KIND_STREAM_REMINDER, "stream_reminder"),
        (KIND_STREAM_MESSAGE_DIFF, "stream_message_diff"),
        (KIND_FORUM_POST, "forum_post"),
        (KIND_FORUM_VOTE, "forum_vote"),
        (KIND_FORUM_COMMENT, "forum_comment"),
    ] {
        registry.register(static_kind(
            kind,
            name,
            Scope::MessagesWrite,
            ChannelRequired,
        ));
    }

    // --- NIP-29 admin kinds --------------------------------------------------
    registry.register(static_kind(
        KIND_NIP29_PUT_USER,
        "nip29_put_user",
        Scope::AdminChannels,
        ChannelRequired,
    ));
    registry.register(static_kind(
        KIND_NIP29_REMOVE_USER,
        "nip29_remove_user",
        Scope::AdminChannels,
        ChannelRequired,
    ));
    registry.register(static_kind(
        KIND_NIP29_DELETE_GROUP,
        "nip29_delete_group",
        Scope::AdminChannels,
        ChannelRequired,
    ));
    // kind:9002 — scope depends on the `archived` tag (see `EditMetadataExt`).
    registry.register(KindDescriptor {
        kind: KIND_NIP29_EDIT_METADATA,
        name: "nip29_edit_metadata",
        required_scope: RequiredScope::Dynamic,
        scoping: ChannelRequired,
        read_gate: ReadGate::Public,
        authorship: Authorship::ClientWritable,
        extension: Some(&EDIT_METADATA_EXT),
    });

    // --- NIP-43: relay admin commands (global, AdminUsers) ------------------
    for (kind, name) in [
        (RELAY_ADMIN_ADD_MEMBER, "relay_admin_add_member"),
        (RELAY_ADMIN_REMOVE_MEMBER, "relay_admin_remove_member"),
        (RELAY_ADMIN_CHANGE_ROLE, "relay_admin_change_role"),
        (
            RELAY_ADMIN_SET_WORKSPACE_PROFILE,
            "relay_admin_set_workspace_profile",
        ),
    ] {
        registry.register(static_kind(kind, name, Scope::AdminUsers, Global));
    }

    // --- NIP-IA: identity archive/unarchive requests -------------------------
    registry.register(static_kind(
        KIND_IA_ARCHIVE_REQUEST,
        "ia_archive_request",
        Scope::UsersWrite,
        Global,
    ));
    registry.register(static_kind(
        KIND_IA_UNARCHIVE_REQUEST,
        "ia_unarchive_request",
        Scope::UsersWrite,
        Global,
    ));

    // --- Channel creation / canvas / join / leave ----------------------------
    registry.register(static_kind(
        KIND_NIP29_CREATE_GROUP,
        "nip29_create_group",
        Scope::ChannelsWrite,
        ChannelOptional,
    ));
    registry.register(static_kind(
        KIND_CANVAS,
        "canvas",
        Scope::ChannelsWrite,
        ChannelRequired,
    ));
    registry.register(static_kind(
        KIND_NIP29_JOIN_REQUEST,
        "nip29_join_request",
        Scope::ChannelsRead,
        ChannelOptional,
    ));
    registry.register(static_kind(
        KIND_NIP29_LEAVE_REQUEST,
        "nip29_leave_request",
        Scope::ChannelsRead,
        ChannelRequired,
    ));
    registry.register(static_kind(
        KIND_NIP43_LEAVE_REQUEST,
        "nip43_leave_request",
        Scope::ChannelsRead,
        Global,
    ));

    // --- Huddle lifecycle + guidelines ---------------------------------------
    for (kind, name) in [
        (KIND_HUDDLE_STARTED, "huddle_started"),
        (KIND_HUDDLE_PARTICIPANT_JOINED, "huddle_participant_joined"),
        (KIND_HUDDLE_PARTICIPANT_LEFT, "huddle_participant_left"),
        (KIND_HUDDLE_ENDED, "huddle_ended"),
        (KIND_HUDDLE_GUIDELINES, "huddle_guidelines"),
    ] {
        registry.register(static_kind(
            kind,
            name,
            Scope::ChannelsWrite,
            ChannelRequired,
        ));
    }

    // --- NIP-34: git repository + project events (global, ReposWrite) -------
    for (kind, name) in [
        (KIND_GIT_REPO_ANNOUNCEMENT, "git_repo_announcement"),
        (KIND_GIT_REPO_STATE, "git_repo_state"),
        (KIND_PROJECT, "project"),
    ] {
        registry.register(static_kind(kind, name, Scope::ReposWrite, Global));
    }

    // --- NIP-34: git activity events (global, MessagesWrite) -----------------
    for (kind, name) in [
        (KIND_GIT_PATCH, "git_patch"),
        (KIND_GIT_PULL_REQUEST, "git_pull_request"),
        (KIND_GIT_PR_UPDATE, "git_pr_update"),
        (KIND_GIT_ISSUE, "git_issue"),
        (KIND_GIT_STATUS_OPEN, "git_status_open"),
        (KIND_GIT_STATUS_MERGED, "git_status_merged"),
        (KIND_GIT_STATUS_CLOSED, "git_status_closed"),
        (KIND_GIT_STATUS_DRAFT, "git_status_draft"),
    ] {
        registry.register(static_kind(kind, name, Scope::MessagesWrite, Global));
    }

    // --- Command kinds: DM management, workflows, approvals ------------------
    for (kind, name) in [
        (KIND_DM_OPEN, "dm_open"),
        (KIND_DM_ADD_MEMBER, "dm_add_member"),
        (KIND_DM_HIDE, "dm_hide"),
    ] {
        registry.register(static_kind(
            kind,
            name,
            Scope::MessagesWrite,
            ChannelOptional,
        ));
    }
    for (kind, name) in [
        (KIND_WORKFLOW_DEF, "workflow_def"),
        (KIND_WORKFLOW_TRIGGER, "workflow_trigger"),
        (KIND_APPROVAL_GRANT, "approval_grant"),
        (KIND_APPROVAL_DENY, "approval_deny"),
    ] {
        registry.register(static_kind(
            kind,
            name,
            Scope::MessagesWrite,
            ChannelOptional,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built() -> KindRegistry {
        let mut registry = KindRegistry::new();
        register_builtin(&mut registry);
        registry
    }

    #[test]
    fn register_builtin_has_no_duplicate_kinds() {
        // `register_builtin` itself would have panicked already on a
        // duplicate; this just proves it completes and yields more than a
        // handful of entries.
        let registry = built();
        assert!(registry.iter().count() > 60);
    }

    #[test]
    fn author_only_kinds_match_buzz_core() {
        let registry = built();
        for &kind in buzz_core::kind::AUTHOR_ONLY_KINDS {
            // `KIND_PUSH_LEASE` is deliberately unregistered in this PR (see
            // module docs) — the relay keeps it inline.
            if kind == buzz_core::kind::KIND_PUSH_LEASE {
                continue;
            }
            let descriptor = registry
                .get(kind)
                .unwrap_or_else(|| panic!("author-only kind {kind} must be registered"));
            assert_eq!(
                descriptor.read_gate,
                ReadGate::AuthorOnly,
                "kind {kind} is in AUTHOR_ONLY_KINDS but not registered ReadGate::AuthorOnly"
            );
        }
    }

    #[test]
    fn edit_metadata_scope_resolves_from_archived_tag() {
        let registry = built();
        let descriptor = registry.get(KIND_NIP29_EDIT_METADATA).unwrap();
        assert!(matches!(descriptor.required_scope, RequiredScope::Dynamic));
        let ext = descriptor
            .extension
            .expect("edit-metadata must have an extension");

        let no_tag = nostr::EventBuilder::new(nostr::Kind::Custom(9002_u16), "")
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();
        assert_eq!(ext.required_scope(&no_tag), Scope::ChannelsWrite);

        let archived = nostr::EventBuilder::new(nostr::Kind::Custom(9002_u16), "")
            .tag(nostr::Tag::custom(
                nostr::TagKind::Custom("archived".into()),
                ["true"],
            ))
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap();
        assert_eq!(ext.required_scope(&archived), Scope::AdminChannels);
    }
}
