//! Closed SQL vocabulary: caller input is always bound, never interpolated.

pub(super) struct Operation {
    pub table: &'static str,
    pub action: &'static str,
    pub selector: &'static str,
    pub key: &'static str,
    pub identity: &'static str,
    // Complete literal UPDATE prefixes keep the table and SET list auditable.
    pub update: Option<&'static str>,
}

impl Operation {
    // Native primary-key ordering lets PostgreSQL page without sorting JSON
    // representations of every remaining key on every batch.
    pub(super) fn pagination(&self) -> (&'static str, &'static str) {
        match self.table {
            "api_tokens" | "relay_invites" | "product_feedback" | "push_wake_outbox" | "operator_listener_outbox" => ("id", "id>($3->>'id')::uuid"),
            "events" => ("created_at,id", "(created_at,id)>(($3->>'created_at')::timestamptz,decode($3->>'id','hex'))"),
            "event_mentions" => ("pubkey_hex,event_id", "(pubkey_hex,event_id)>($3->>'pubkey_hex',decode($3->>'event_id','hex'))"),
            "push_match_queue" => ("event_id", "event_id>decode($3->>'event_id','hex')"),
            "operator_listener_pubkeys" => ("listener_pubkey,target_pubkey", "(listener_pubkey,target_pubkey)>(decode($3->>'listener_pubkey','hex'),decode($3->>'target_pubkey','hex'))"),
            "channel_members" => ("channel_id,pubkey", "(channel_id,pubkey)>(($3->>'channel_id')::uuid,decode($3->>'pubkey','hex'))"),
            "join_policy_acceptances" => ("pubkey,policy_version", "(pubkey,policy_version)>($3->>'pubkey',$3->>'policy_version')"),
            "pubkey_allowlist" | "relay_operators" => ("pubkey", "pubkey>decode($3->>'pubkey','hex')"),
            "personal_read_frontiers" => ("actor,channel_id,root_id", "(actor,channel_id,root_id)>(decode($3->>'actor','hex'),($3->>'channel_id')::uuid,decode($3->>'root_id','hex'))"),
            "personal_read_accounts" => ("actor", "actor>decode($3->>'actor','hex')"),
            // The only remaining operations have a text pubkey primary key.
            _ => ("pubkey", "pubkey>$3->>'pubkey'"),
        }
    }
}

const ID_KEY: &str = "jsonb_build_object('id',id)";
const ID_MATCH: &str = "id=($3->>'id')::uuid";
const BYTE_KEY: &str = "jsonb_build_object('pubkey',encode(pubkey,'hex'))";
const BYTE_MATCH: &str = "pubkey=decode($3->>'pubkey','hex')";
const TEXT_KEY: &str = "jsonb_build_object('pubkey',pubkey)";
const TEXT_MATCH: &str = "pubkey=$3->>'pubkey'";
const MEMBER_KEY: &str =
    "jsonb_build_object('channel_id',channel_id,'pubkey',encode(pubkey,'hex'))";
const MEMBER_MATCH: &str =
    "channel_id=($3->>'channel_id')::uuid AND pubkey=decode($3->>'pubkey','hex')";

const fn delete(
    table: &'static str,
    selector: &'static str,
    key: &'static str,
    identity: &'static str,
) -> Operation {
    Operation {
        table,
        action: "delete",
        selector,
        key,
        identity,
        update: None,
    }
}

const fn clear(
    table: &'static str,
    action: &'static str,
    selector: &'static str,
    key: &'static str,
    identity: &'static str,
    update: &'static str,
) -> Operation {
    Operation {
        table,
        action,
        selector,
        key,
        identity,
        update: Some(update),
    }
}

// Only directly authored personal-state kinds. Shared emoji sets/entries,
// profile events, DM commands and relay-signed membership snapshots are not
// personal settings. Include soft-deleted revisions: they still hold payloads.
macro_rules! personal_events {
    () => { "pubkey=decode($2,'hex') AND kind IN (3,10000,10001,10002,10003,10030,30000,30003,30315,30078)" };
}
macro_rules! personal_event_copy {
    () => {
        concat!(
            "event_id IN (SELECT id FROM events WHERE community_id=$1 AND ",
            personal_events!(),
            ")"
        )
    };
}

// Children before parents. Memberships include removed rows; credentials include
// expired/revoked rows. Nullable provenance updates never remove another owner.
pub(super) const OPERATIONS: &[Operation] = &[
    delete("product_feedback", "submitter_pubkey=decode($2,'hex')", ID_KEY, ID_MATCH),
    delete("api_tokens", "owner_pubkey=decode($2,'hex')", ID_KEY, ID_MATCH),
    delete("relay_invites", "created_by=$2", ID_KEY, ID_MATCH),
    delete("channel_members", "pubkey=decode($2,'hex')", MEMBER_KEY, MEMBER_MATCH),
    delete("join_policy_acceptances", "pubkey=$2",
        "jsonb_build_object('pubkey',pubkey,'policy_version',policy_version)",
        "pubkey=$3->>'pubkey' AND policy_version=$3->>'policy_version'"),
    delete("relay_members", "pubkey=$2", TEXT_KEY, TEXT_MATCH),
    delete("pubkey_allowlist", "pubkey=decode($2,'hex')", BYTE_KEY, BYTE_MATCH),
    delete("personal_read_frontiers", "actor=decode($2,'hex')",
        "jsonb_build_object('actor',encode(actor,'hex'),'channel_id',channel_id,'root_id',encode(root_id,'hex'))",
        "actor=decode($3->>'actor','hex') AND channel_id=($3->>'channel_id')::uuid AND root_id=decode($3->>'root_id','hex')"),
    delete("personal_read_accounts", "actor=decode($2,'hex')",
        "jsonb_build_object('actor',encode(actor,'hex'))", "actor=decode($3->>'actor','hex')"),
    clear("api_tokens", "clear_revoked_by", "revoked_by=decode($2,'hex') AND owner_pubkey<>decode($2,'hex')", ID_KEY, ID_MATCH, "UPDATE api_tokens SET revoked_by=NULL"),
    clear("channel_members", "clear_invited_by", "invited_by=decode($2,'hex') AND pubkey<>decode($2,'hex')", MEMBER_KEY, MEMBER_MATCH,
        "UPDATE channel_members SET invited_by=NULL,sync_modified_at=clock_timestamp()"),
    clear("channel_members", "clear_removed_by", "removed_by=decode($2,'hex') AND pubkey<>decode($2,'hex')", MEMBER_KEY, MEMBER_MATCH,
        "UPDATE channel_members SET removed_by=NULL,sync_modified_at=clock_timestamp()"),
    clear("relay_members", "clear_added_by", "added_by=$2 AND pubkey<>$2", TEXT_KEY, TEXT_MATCH,
        "UPDATE relay_members SET added_by=NULL,updated_at=clock_timestamp()"),
    clear("pubkey_allowlist", "clear_added_by", "added_by=decode($2,'hex') AND pubkey<>decode($2,'hex')", BYTE_KEY, BYTE_MATCH, "UPDATE pubkey_allowlist SET added_by=NULL"),
    clear("archived_identities", "clear_replaced_by", "replaced_by=$2", TEXT_KEY, TEXT_MATCH, "UPDATE archived_identities SET replaced_by=NULL"),
    // Copies before their source, so interruption never loses attribution.
    delete("event_mentions", personal_event_copy!(),
        "jsonb_build_object('pubkey_hex',pubkey_hex,'event_id',encode(event_id,'hex'))",
        "pubkey_hex=$3->>'pubkey_hex' AND event_id=decode($3->>'event_id','hex')"),
    delete("push_match_queue", personal_event_copy!(),
        "jsonb_build_object('event_id',encode(event_id,'hex'))", "event_id=decode($3->>'event_id','hex')"),
    delete("push_wake_outbox", personal_event_copy!(), ID_KEY, ID_MATCH),
    delete("operator_listener_outbox", personal_event_copy!(), ID_KEY, ID_MATCH),
    delete("events", personal_events!(),
        "jsonb_build_object('created_at',created_at,'id',encode(id,'hex'))",
        "created_at=($3->>'created_at')::timestamptz AND id=decode($3->>'id','hex')"),
];

// No community invocation ever executes these deployment-global selectors.
pub(super) const GLOBAL_OPERATIONS: &[Operation] = &[
    delete("relay_operators", "pubkey=decode($2,'hex')", BYTE_KEY, BYTE_MATCH),
    delete("operator_listener_pubkeys", "listener_pubkey=decode($2,'hex') OR target_pubkey=decode($2,'hex')",
        "jsonb_build_object('listener_pubkey',encode(listener_pubkey,'hex'),'target_pubkey',encode(target_pubkey,'hex'))",
        "listener_pubkey=decode($3->>'listener_pubkey','hex') AND target_pubkey=decode($3->>'target_pubkey','hex')"),
    delete("product_feedback", "submitter_pubkey=decode($2,'hex')", ID_KEY, ID_MATCH),
];
