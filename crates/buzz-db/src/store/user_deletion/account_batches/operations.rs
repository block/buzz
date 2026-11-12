//! Closed SQL vocabulary: caller input is always bound, never interpolated.

pub(super) struct Operation {
    pub table: &'static str,
    pub action: &'static str,
    pub selector: &'static str,
    pub key: &'static str,
    pub identity: &'static str,
    pub update: Option<&'static str>,
}

impl Operation {
    // Native primary-key ordering lets PostgreSQL page without sorting JSON
    // representations of every remaining key on every batch.
    pub(super) fn pagination(&self) -> (&'static str, &'static str) {
        match self.table {
            "api_tokens" | "relay_invites" => ("id", "id>($3->>'id')::uuid"),
            "channel_members" => ("channel_id,pubkey", "(channel_id,pubkey)>(($3->>'channel_id')::uuid,decode($3->>'pubkey','hex'))"),
            "join_policy_acceptances" => ("pubkey,policy_version", "(pubkey,policy_version)>($3->>'pubkey',$3->>'policy_version')"),
            "pubkey_allowlist" => ("pubkey", "pubkey>decode($3->>'pubkey','hex')"),
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

// Children before parents. Memberships include removed rows; credentials include
// expired/revoked rows. Nullable provenance updates never remove another owner.
pub(super) const OPERATIONS: &[Operation] = &[
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
    clear("api_tokens", "clear_revoked_by", "revoked_by=decode($2,'hex') AND owner_pubkey<>decode($2,'hex')", ID_KEY, ID_MATCH, "revoked_by=NULL"),
    clear("channel_members", "clear_invited_by", "invited_by=decode($2,'hex') AND pubkey<>decode($2,'hex')", MEMBER_KEY, MEMBER_MATCH,
        "invited_by=NULL,sync_modified_at=clock_timestamp()"),
    clear("channel_members", "clear_removed_by", "removed_by=decode($2,'hex') AND pubkey<>decode($2,'hex')", MEMBER_KEY, MEMBER_MATCH,
        "removed_by=NULL,sync_modified_at=clock_timestamp()"),
    clear("relay_members", "clear_added_by", "added_by=$2 AND pubkey<>$2", TEXT_KEY, TEXT_MATCH,
        "added_by=NULL,updated_at=clock_timestamp()"),
    clear("pubkey_allowlist", "clear_added_by", "added_by=decode($2,'hex') AND pubkey<>decode($2,'hex')", BYTE_KEY, BYTE_MATCH, "added_by=NULL"),
    clear("archived_identities", "clear_replaced_by", "replaced_by=$2", TEXT_KEY, TEXT_MATCH, "replaced_by=NULL"),
];
