//! The community's landing channel and its welcome message.
//!
//! All three live on the community's `communities` row (`landing_channel_id`,
//! `welcome_message`, `welcome_title`), so a community has at most one
//! landing channel by construction and a move is a single-row update. NIP-11
//! `landing_channel` is the authoritative read; the landing channel's
//! kind:39000 carries a projection of it
//! ([`CommunityLanding::group_metadata_tags`]).
//!
//! Lock order: a writer that locks both a channel row and the `communities`
//! row locks the channel row first. Moving the designation to a channel
//! therefore needs that channel's row lock, which every write that could
//! make the channel ineligible also holds.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::channel::{ChannelType, ChannelUpdate, ChannelVisibility, NewChannel};
use crate::{AdmittedTx, Db, DbError, Result};

/// Refusal for archiving, making private, expiring or deleting the landing
/// channel.
pub const LANDING_PROTECTED: &str =
    "this is the community's landing channel; make another channel the landing channel first";

/// Refusal for designating a channel that is not an open, unarchived stream
/// without a TTL, judged on the state the request leaves behind.
pub const LANDING_INELIGIBLE: &str = "landing requires an open, unarchived channel without a ttl";

/// Refusal for a welcome message or title sent to a channel that is not, and
/// does not become, the landing channel.
pub const WELCOME_NOT_LANDING: &str =
    "welcome and welcome_title can only be set on the landing channel";

/// The longest welcome message, in Unicode scalar values. The message is
/// Markdown source, which the relay stores as-is; rendering it is a client
/// concern.
pub const WELCOME_MAX_CHARS: usize = 4000;

/// The longest welcome title, in Unicode scalar values.
pub const WELCOME_TITLE_MAX_CHARS: usize = 120;

/// The name of the landing channel the relay adopts or creates.
pub const LANDING_CHANNEL_NAME: &str = "general";

/// A community's landing designation and custom welcome message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommunityLanding {
    /// The landing channel, if one is designated.
    pub channel_id: Option<Uuid>,
    /// The custom welcome message (Markdown source); `None` means the client
    /// default.
    pub welcome: Option<String>,
    /// The custom welcome title; `None` means the client default.
    pub welcome_title: Option<String>,
}

impl CommunityLanding {
    /// The kind:39000 tags for `channel_id`: a bare `["landing"]` while it is
    /// the landing channel, then `["welcome", text]` and
    /// `["welcome_title", text]` while each is customized. Empty for every
    /// other channel, so a re-emission after a move drops all three from the
    /// previous landing channel.
    pub fn group_metadata_tags(&self, channel_id: Uuid) -> Vec<Vec<String>> {
        if self.channel_id != Some(channel_id) {
            return Vec::new();
        }
        let mut tags = vec![vec!["landing".to_owned()]];
        for (name, text) in [
            ("welcome", &self.welcome),
            ("welcome_title", &self.welcome_title),
        ] {
            if let Some(text) = text.as_deref().filter(|t| !t.is_empty()) {
                tags.push(vec![name.to_owned(), text.to_owned()]);
            }
        }
        tags
    }
}

/// Whether `c` renders as nothing on its own: format characters, fillers and
/// blank glyphs that are not Unicode `White_Space`. A welcome message or
/// title made only of these and whitespace is blank ([`is_blank`]); among
/// visible text they are allowed, like the zero-width joiner in an emoji
/// sequence.
///
/// The set is U+00AD, U+034F, U+061C, U+115F, U+1160, U+17B4, U+17B5,
/// U+180B–U+180F, U+200B–U+200F, U+202A–U+202E, U+2060–U+206F, U+2800,
/// U+3164, U+FE00–U+FE0F, U+FEFF, U+FFA0, U+FFF9–U+FFFB, U+1BCA0–U+1BCA3,
/// U+1D173–U+1D17A, U+E0000–U+E007F and U+E0100–U+E01EF. NIP-CL publishes
/// it and clients mirror it character for character, so change all of them
/// together.
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'
            | '\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{2800}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E007F}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

/// Whether `c` is a bidirectional formatting control, which can reorder how
/// the text around it is displayed. Welcome messages and titles refuse them
/// anywhere.
///
/// The set is U+061C, U+200E, U+200F, U+202A–U+202E and U+2066–U+2069. Every
/// one is also [`is_invisible`], so a value of bidi controls alone is
/// refused as blank. NIP-CL publishes the set and clients mirror it character
/// for character, so change all of them together.
pub fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Whether every character of `text` is Unicode `White_Space` (what
/// [`str::trim`] strips) or [`is_invisible`], so it would render as an empty
/// heading or card. The empty string is blank.
pub fn is_blank(text: &str) -> bool {
    text.chars().all(|c| c.is_whitespace() || is_invisible(c))
}

/// Why `text` cannot be a custom welcome message, or `None` when it can.
/// The rules apply in order: not [`is_blank`], no [`is_bidi_control`], at
/// most [`WELCOME_MAX_CHARS`], then no control characters. The message is
/// Markdown source, so line breaks and tabs are allowed; no other control
/// character is. The caller handles the empty string, which restores the
/// default.
pub fn welcome_message_error(text: &str) -> Option<&'static str> {
    if is_blank(text) {
        Some("welcome message must not be blank")
    } else if text.chars().any(is_bidi_control) {
        Some("welcome message must not contain bidirectional control characters")
    } else if text.chars().count() > WELCOME_MAX_CHARS {
        Some("welcome message is too long (max 4000 characters)")
    } else if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        Some("welcome message must not contain control characters other than line breaks and tabs")
    } else {
        None
    }
}

/// Why `text` cannot be a custom welcome title, or `None` when it can: one
/// line of text, with no control character and no Unicode line or paragraph
/// separator (U+2028, U+2029). The rules apply in order: not [`is_blank`],
/// no [`is_bidi_control`], at most [`WELCOME_TITLE_MAX_CHARS`], then the
/// single-line rule. The caller handles the empty string, which restores the
/// default.
pub fn welcome_title_error(text: &str) -> Option<&'static str> {
    if is_blank(text) {
        Some("welcome title must not be blank")
    } else if text.chars().any(is_bidi_control) {
        Some("welcome title must not contain bidirectional control characters")
    } else if text.chars().count() > WELCOME_TITLE_MAX_CHARS {
        Some("welcome title is too long (max 120 characters)")
    } else if text
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        Some("welcome title must be a single line without control characters")
    } else {
        None
    }
}

/// What [`Db::ensure_landing_channel`] found or did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandingAdoption {
    /// A channel was already designated.
    Existing(Uuid),
    /// A channel was already designated but is no longer eligible: archived,
    /// private, deleted or given a TTL past the relay's protections (a
    /// direct write, or a relay from before them). It stays designated; an
    /// owner or admin moves it with kind:9002 `landing`.
    ExistingIneligible(Uuid),
    /// An eligible channel named `general` became the landing channel.
    Reused(Uuid),
    /// The relay created `general`, owned by `steward`, as the landing
    /// channel.
    Created {
        /// The new channel.
        channel_id: Uuid,
        /// Its owner: the community's earliest owner, else earliest admin.
        steward: [u8; 32],
    },
    /// No eligible `general` exists and the community has no owner or admin
    /// to own a new one. The designation stays empty.
    SkippedNoSteward,
    /// No eligible `general` exists and the name is held by a listed channel
    /// that cannot be the landing channel (archived, a forum, or with a TTL).
    SkippedNameTaken,
}

impl LandingAdoption {
    /// The landing channel after the call, if any.
    pub fn channel_id(&self) -> Option<Uuid> {
        match *self {
            Self::Existing(id) | Self::ExistingIneligible(id) | Self::Reused(id) => Some(id),
            Self::Created { channel_id, .. } => Some(channel_id),
            Self::SkippedNoSteward | Self::SkippedNameTaken => None,
        }
    }

    /// The `outcome` label of `buzz_landing_adoption_total`, or `None` when
    /// a designation already existed.
    pub fn outcome(&self) -> Option<&'static str> {
        match self {
            Self::Existing(_) => None,
            Self::ExistingIneligible(_) => Some("existing_ineligible"),
            Self::Reused(_) => Some("reused"),
            Self::Created { .. } => Some("created"),
            Self::SkippedNoSteward => Some("skipped_no_steward"),
            Self::SkippedNameTaken => Some("skipped_name_taken"),
        }
    }
}

async fn read_landing(
    conn: &mut PgConnection,
    community_id: CommunityId,
) -> Result<CommunityLanding> {
    let row: Option<(Option<Uuid>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT landing_channel_id, welcome_message, welcome_title FROM communities WHERE id = $1",
    )
    .bind(community_id.as_uuid())
    .fetch_optional(conn)
    .await?;
    let (channel_id, welcome, welcome_title) = row.unwrap_or_default();
    Ok(CommunityLanding {
        channel_id,
        welcome: welcome.filter(|w| !w.is_empty()),
        welcome_title: welcome_title.filter(|t| !t.is_empty()),
    })
}

/// Locks the community row on `tx` and returns its landing channel. Taken
/// after any channel row lock (see the module docs).
pub(crate) async fn lock_designation(tx: &mut AdmittedTx) -> Result<Option<Uuid>> {
    let community_id = tx.community();
    Ok(sqlx::query_scalar(
        "SELECT landing_channel_id FROM communities WHERE id = $1 FOR NO KEY UPDATE",
    )
    .bind(community_id.as_uuid())
    .fetch_one(tx.conn())
    .await?)
}

/// Makes `channel_id` the landing channel on `tx`, which holds the community
/// row lock ([`lock_designation`]).
pub(crate) async fn set_designation(tx: &mut AdmittedTx, channel_id: Uuid) -> Result<()> {
    let community_id = tx.community();
    sqlx::query("UPDATE communities SET landing_channel_id = $2 WHERE id = $1")
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .execute(tx.conn())
        .await?;
    Ok(())
}

/// Writes the welcome message and title on `tx`, which holds the community
/// row lock ([`lock_designation`]), in one statement. Each is a partial
/// update: `None` leaves it unchanged, `Some(None)` restores the default and
/// `Some(Some(text))` sets it.
pub(crate) async fn set_welcome(
    tx: &mut AdmittedTx,
    message: Option<Option<&str>>,
    title: Option<Option<&str>>,
) -> Result<()> {
    if message.is_none() && title.is_none() {
        return Ok(());
    }
    let community_id = tx.community();
    sqlx::query(
        "UPDATE communities SET \
         welcome_message = CASE WHEN $2 THEN $3 ELSE welcome_message END, \
         welcome_title = CASE WHEN $4 THEN $5 ELSE welcome_title END \
         WHERE id = $1",
    )
    .bind(community_id.as_uuid())
    .bind(message.is_some())
    .bind(message.flatten())
    .bind(title.is_some())
    .bind(title.flatten())
    .execute(tx.conn())
    .await?;
    Ok(())
}

/// Refuses with [`LANDING_PROTECTED`] when `channel_id` is the landing
/// channel. `tx` holds the channel's row lock, so the designation cannot
/// move to the channel meanwhile; a plain read is enough.
pub(crate) async fn refuse_if_landing(tx: &mut AdmittedTx, channel_id: Uuid) -> Result<()> {
    let community_id = tx.community();
    let landing: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT landing_channel_id FROM communities WHERE id = $1")
            .bind(community_id.as_uuid())
            .fetch_optional(tx.conn())
            .await?;
    if landing.flatten() == Some(channel_id) {
        return Err(DbError::AccessDenied(LANDING_PROTECTED.to_string()));
    }
    Ok(())
}

/// SQL that is true for a channel that may be the landing channel.
const ELIGIBLE_SQL: &str = "channel_type = 'stream' AND visibility = 'open' \
     AND archived_at IS NULL AND deleted_at IS NULL AND ttl_seconds IS NULL";

/// See [`Db::ensure_landing_channel`].
async fn ensure_landing_channel(
    pool: &PgPool,
    community_id: CommunityId,
) -> Result<LandingAdoption> {
    let current: Option<(Uuid, bool)> = {
        let mut connection = crate::observability::acquire_writer(
            pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        // The designation and, in the same read, whether it is still eligible.
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT c.landing_channel_id, EXISTS (SELECT 1 FROM channels \
             WHERE community_id = c.id AND id = c.landing_channel_id AND {ELIGIBLE_SQL}) \
             FROM communities c WHERE c.id = $1 AND c.landing_channel_id IS NOT NULL"
        )))
        .bind(community_id.as_uuid())
        .fetch_optional(&mut *connection)
        .await?
    };
    match current {
        Some((channel_id, true)) => return Ok(LandingAdoption::Existing(channel_id)),
        Some((channel_id, false)) => return Ok(LandingAdoption::ExistingIneligible(channel_id)),
        None => {}
    }
    if let Some(adopted) = adopt_general(pool, community_id).await? {
        return Ok(adopted);
    }
    let Some(steward) = steward(pool, community_id).await? else {
        return Ok(LandingAdoption::SkippedNoSteward);
    };
    // Read whether a listed channel that cannot be adopted (archived, a
    // forum, or with a TTL) holds the name first: the common steady state of
    // such a community is then one read, not a write transaction that
    // `idx_channels_listed_name` rolls back. An eligible holder is left to
    // `create_general`: one that a concurrent ensure created and designated
    // since `adopt_general` read is found by its locked designation re-check.
    if general_name_held_by_ineligible(pool, community_id).await? {
        return Ok(LandingAdoption::SkippedNameTaken);
    }
    match create_general(pool, community_id, steward).await {
        // A concurrent creator, or a rename, took the name since the read:
        // adopt its channel if it is eligible.
        Err(DbError::ChannelNameTaken(_)) => Ok(adopt_general(pool, community_id)
            .await?
            .unwrap_or(LandingAdoption::SkippedNameTaken)),
        created => created,
    }
}

/// Whether a listed channel that cannot be the landing channel holds the
/// name `general`: the predicate of `idx_channels_listed_name`, which
/// `create_general` would trip, without the eligible channels.
async fn general_name_held_by_ineligible(pool: &PgPool, community_id: CommunityId) -> Result<bool> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let held: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT EXISTS (SELECT 1 FROM channels \
         WHERE community_id = $1 AND lower(name) = $2 \
           AND visibility = 'open' AND NOT unlisted \
           AND channel_type IN ('stream', 'forum') AND deleted_at IS NULL \
           AND NOT ({ELIGIBLE_SQL}))"
    )))
    .bind(community_id.as_uuid())
    .bind(LANDING_CHANNEL_NAME)
    .fetch_one(&mut *connection)
    .await?;
    Ok(held)
}

/// Designates the oldest eligible `general`, preferring a listed one.
/// `None` when there is none.
async fn adopt_general(
    pool: &PgPool,
    community_id: CommunityId,
) -> Result<Option<LandingAdoption>> {
    let candidate: Option<Uuid> = {
        let mut connection = crate::observability::acquire_writer(
            pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT id FROM channels WHERE community_id = $1 AND lower(name) = $2 \
             AND {ELIGIBLE_SQL} ORDER BY unlisted ASC, created_at ASC, id ASC LIMIT 1"
        )))
        .bind(community_id.as_uuid())
        .bind(LANDING_CHANNEL_NAME)
        .fetch_optional(&mut *connection)
        .await?
    };
    let Some(candidate) = candidate else {
        return Ok(None);
    };

    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let mut locked = match crate::channel::lock_channel_for_settings_write(
        &mut tx,
        candidate,
        &ChannelUpdate::default(),
    )
    .await
    {
        Ok(locked) => locked,
        Err(DbError::ChannelNotFound(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    // Under the row lock: still eligible, and still named `general`.
    let eligible: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT EXISTS (SELECT 1 FROM channels WHERE community_id = $1 AND id = $2 \
         AND lower(name) = $3 AND {ELIGIBLE_SQL})"
    )))
    .bind(community_id.as_uuid())
    .bind(candidate)
    .bind(LANDING_CHANNEL_NAME)
    .fetch_one(locked.tx().conn())
    .await?;
    if !eligible {
        return Ok(None);
    }
    if let Some(existing) = lock_designation(locked.tx()).await? {
        return Ok(Some(LandingAdoption::Existing(existing)));
    }
    set_designation(locked.tx(), candidate).await?;
    tx.commit().await?;
    Ok(Some(LandingAdoption::Reused(candidate)))
}

/// The community's earliest owner, else its earliest admin.
async fn steward(pool: &PgPool, community_id: CommunityId) -> Result<Option<[u8; 32]>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    let pubkey: Option<String> = sqlx::query_scalar(
        "SELECT pubkey FROM relay_members \
         WHERE community_id = $1 AND role IN ('owner', 'admin') \
         ORDER BY role = 'owner' DESC, created_at ASC, pubkey ASC LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .fetch_optional(&mut *connection)
    .await?;
    pubkey
        .map(|hex_key| {
            hex::decode(&hex_key)
                .ok()
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .ok_or_else(|| {
                    DbError::InvalidData(format!(
                        "relay member pubkey is not 32-byte hex: {hex_key}"
                    ))
                })
        })
        .transpose()
}

/// Creates a listed, open `general` stream owned by `steward` and designates
/// it, unless a designation appeared meanwhile.
async fn create_general(
    pool: &PgPool,
    community_id: CommunityId,
    steward: [u8; 32],
) -> Result<LandingAdoption> {
    let mut tx = crate::begin_community_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    // The community row first: the new channel row is no one else's lock
    // yet, so this keeps the channel-then-community order for every
    // existing channel.
    if let Some(existing) = lock_designation(&mut tx).await? {
        return Ok(LandingAdoption::Existing(existing));
    }
    let channel_id = Uuid::new_v4();
    crate::channel::insert_channel_rows(
        &mut tx,
        channel_id,
        NewChannel {
            name: LANDING_CHANNEL_NAME,
            channel_type: ChannelType::Stream,
            visibility: ChannelVisibility::Open,
            description: None,
            created_by: &steward,
            ttl_seconds: None,
            labels: &[],
            unlisted: false,
            members: &[],
        },
    )
    .await?;
    set_designation(&mut tx, channel_id).await?;
    tx.commit().await?;
    Ok(LandingAdoption::Created {
        channel_id,
        steward,
    })
}

impl Db {
    /// Returns the community's landing channel and welcome message and
    /// title.
    #[datastore_span(name = "get_community_landing", system = "postgresql")]
    pub async fn get_community_landing(
        &self,
        community_id: CommunityId,
    ) -> Result<CommunityLanding> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        read_landing(&mut connection, community_id).await
    }

    /// Makes sure the community has a landing channel and returns what it
    /// found or did. Idempotent; costs one primary-key read once a channel
    /// is designated.
    ///
    /// With no designation, the oldest eligible (open, unarchived stream
    /// without a TTL) channel named `general` is adopted, a listed one before
    /// an unlisted one. With none, a listed, open `general` is created, owned
    /// by the community's earliest owner, else earliest admin
    /// ([`LandingAdoption::SkippedNoSteward`] when there is neither). If the
    /// name is held by a listed channel that cannot be adopted (an archived
    /// `general`, a forum, or one with a TTL), nothing is created
    /// ([`LandingAdoption::SkippedNameTaken`]). A private or unlisted
    /// `general` does not hold the name, and a private one is never adopted.
    ///
    /// A designated channel that is no longer eligible is reported
    /// ([`LandingAdoption::ExistingIneligible`]) and left designated.
    ///
    /// A NULL designation is the retry record: every call that finds one
    /// tries again.
    #[datastore_span(name = "ensure_landing_channel", system = "postgresql")]
    pub async fn ensure_landing_channel(
        &self,
        community_id: CommunityId,
    ) -> Result<LandingAdoption> {
        ensure_landing_channel(&self.pool, community_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landing_tags_mark_only_the_landing_channel() {
        let (landing, other) = (Uuid::new_v4(), Uuid::new_v4());
        let tag = |parts: &[&str]| parts.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        let custom = CommunityLanding {
            channel_id: Some(landing),
            welcome: Some("Hi all!\nSay hello.".to_string()),
            welcome_title: Some("Welcome aboard".to_string()),
        };
        assert_eq!(
            custom.group_metadata_tags(landing),
            vec![
                tag(&["landing"]),
                tag(&["welcome", "Hi all!\nSay hello."]),
                tag(&["welcome_title", "Welcome aboard"]),
            ]
        );
        assert!(custom.group_metadata_tags(other).is_empty());
        let title_only = CommunityLanding {
            welcome: None,
            ..custom.clone()
        };
        assert_eq!(
            title_only.group_metadata_tags(landing),
            vec![tag(&["landing"]), tag(&["welcome_title", "Welcome aboard"])]
        );
        let default_text = CommunityLanding {
            channel_id: Some(landing),
            welcome: None,
            welcome_title: None,
        };
        assert_eq!(
            default_text.group_metadata_tags(landing),
            vec![tag(&["landing"])]
        );
        assert!(CommunityLanding::default()
            .group_metadata_tags(landing)
            .is_empty());
    }

    #[test]
    fn welcome_message_rules() {
        let at_limit = "a".repeat(WELCOME_MAX_CHARS);
        let over_limit = "é".repeat(WELCOME_MAX_CHARS + 1);
        let over_limit_bidi = format!("{over_limit}\u{202E}");
        let blank = Some("welcome message must not be blank");
        let bidi = Some("welcome message must not contain bidirectional control characters");
        let control = Some(
            "welcome message must not contain control characters other than line breaks and tabs",
        );
        let cases: &[(&str, Option<&str>)] = &[
            ("Welcome!", None),
            ("Hi all!\nIntroduce yourself here.", None),
            (" padded ", None),
            (&at_limit, None),
            // Markdown source: headings, `buzz://` and `nostr:npub` links and
            // tab-indented code are all just text to the relay.
            (
                "# Welcome\n\nSee [#rules](buzz://channel/abc) and nostr:npub1xyz.\n\n\tcode",
                None,
            ),
            ("tab\there", None),
            // Invisible characters are fine beside visible text.
            ("Hello\u{200B}world", None),
            ("\u{200B}Hi\n\tthere\u{FEFF}", None),
            ("Family: 👨\u{200D}👩\u{200D}👧", None),
            ("   ", blank),
            ("\n\n", blank),
            ("\t\n", blank),
            ("\u{200B}", blank),
            ("\u{2800}", blank),
            (" \u{200B} ", blank),
            ("\n\t\u{FEFF}\u{3164}\n", blank),
            ("\u{E0020}\u{1D173}\u{FE0F}", blank),
            // Bidi controls are invisible too: alone they are blank.
            ("\u{202E}", blank),
            ("Hello \u{202E}dlrow", bidi),
            ("Hi\n\u{2066}there\u{2069}", bidi),
            ("mark\u{200F}", bidi),
            // Bidi is checked before length and control characters.
            (&over_limit_bidi, bidi),
            ("\u{202A}bell\u{7}", bidi),
            (
                &over_limit,
                Some("welcome message is too long (max 4000 characters)"),
            ),
            ("windows\r\nline", control),
            ("bell\u{7}", control),
            ("next line\u{85}", control),
        ];
        for (text, expected) in cases {
            assert_eq!(welcome_message_error(text), *expected, "{text:?}");
        }
        // The limit counts scalar values, not bytes: 4000 multi-byte
        // characters fit.
        assert_eq!(welcome_message_error(&"é".repeat(WELCOME_MAX_CHARS)), None);
    }

    #[test]
    fn welcome_title_rules() {
        let at_limit = "é".repeat(WELCOME_TITLE_MAX_CHARS);
        let over_limit = "a".repeat(WELCOME_TITLE_MAX_CHARS + 1);
        let over_limit_bidi = format!("{over_limit}\u{202E}");
        let blank = Some("welcome title must not be blank");
        let bidi = Some("welcome title must not contain bidirectional control characters");
        let control = Some("welcome title must be a single line without control characters");
        let cases: &[(&str, Option<&str>)] = &[
            ("Welcome to Acme", None),
            ("👋 Hi, team!", None),
            (" padded ", None),
            (&at_limit, None),
            // Invisible characters are fine beside visible text.
            ("Hello\u{200B}world", None),
            ("\u{FEFF}Welcome", None),
            ("👨\u{200D}👩\u{200D}👧 Welcome", None),
            ("", blank),
            ("   ", blank),
            ("\n", blank),
            ("\u{200B}", blank),
            ("\u{2800}", blank),
            (" \u{200B} ", blank),
            ("\u{FEFF}", blank),
            ("\u{E0100}", blank),
            ("\u{3164}\u{115F}\u{FFA0}", blank),
            ("\u{00AD}\u{034F}\u{180E}\u{2060}", blank),
            // Bidi controls are invisible too: alone they are blank.
            ("\u{202E}", blank),
            ("\u{2066}\u{2069}", blank),
            ("Hello \u{202E}dlrow", bidi),
            ("\u{2067}Acme\u{2069}", bidi),
            ("mark\u{061C}", bidi),
            // Bidi is checked before length and the single-line rule.
            (&over_limit_bidi, bidi),
            ("Two\u{200E}\nlines", bidi),
            (
                &over_limit,
                Some("welcome title is too long (max 120 characters)"),
            ),
            ("Two\nlines", control),
            ("tab\there", control),
            ("bell\u{7}", control),
            // Not `char::is_control`, but a line break all the same.
            ("line\u{2028}separator", control),
            ("paragraph\u{2029}separator", control),
        ];
        for (text, expected) in cases {
            assert_eq!(welcome_title_error(text), *expected, "{text:?}");
        }
    }

    /// Both ends of every range in the invisible and bidi sets, and the
    /// neighbours just outside them. Clients mirror these sets character for
    /// character, so this pins the exact boundaries.
    #[test]
    fn invisible_and_bidi_sets() {
        // (character, is_invisible, is_bidi_control)
        let cases: &[(char, bool, bool)] = &[
            ('a', false, false),
            (' ', false, false),
            ('\u{00AC}', false, false),
            ('\u{00AD}', true, false),
            ('\u{00AE}', false, false),
            ('\u{034F}', true, false),
            ('\u{061B}', false, false),
            ('\u{061C}', true, true),
            ('\u{061D}', false, false),
            ('\u{115E}', false, false),
            ('\u{115F}', true, false),
            ('\u{1160}', true, false),
            ('\u{1161}', false, false),
            ('\u{17B3}', false, false),
            ('\u{17B4}', true, false),
            ('\u{17B5}', true, false),
            ('\u{17B6}', false, false),
            ('\u{180A}', false, false),
            ('\u{180B}', true, false),
            ('\u{180F}', true, false),
            ('\u{1810}', false, false),
            // U+200A HAIR SPACE is whitespace, not invisible.
            ('\u{200A}', false, false),
            ('\u{200B}', true, false),
            ('\u{200D}', true, false),
            ('\u{200E}', true, true),
            ('\u{200F}', true, true),
            ('\u{2010}', false, false),
            ('\u{2029}', false, false),
            ('\u{202A}', true, true),
            ('\u{202E}', true, true),
            ('\u{202F}', false, false),
            ('\u{205F}', false, false),
            ('\u{2060}', true, false),
            ('\u{2065}', true, false),
            ('\u{2066}', true, true),
            ('\u{2069}', true, true),
            ('\u{206A}', true, false),
            ('\u{206F}', true, false),
            ('\u{2070}', false, false),
            ('\u{27FF}', false, false),
            ('\u{2800}', true, false),
            ('\u{2801}', false, false),
            ('\u{3164}', true, false),
            ('\u{FDFF}', false, false),
            ('\u{FE00}', true, false),
            ('\u{FE0F}', true, false),
            ('\u{FE10}', false, false),
            ('\u{FEFF}', true, false),
            ('\u{FFA0}', true, false),
            ('\u{FFF8}', false, false),
            ('\u{FFF9}', true, false),
            ('\u{FFFB}', true, false),
            ('\u{FFFC}', false, false),
            ('\u{1BC9F}', false, false),
            ('\u{1BCA0}', true, false),
            ('\u{1BCA3}', true, false),
            ('\u{1BCA4}', false, false),
            ('\u{1D172}', false, false),
            ('\u{1D173}', true, false),
            ('\u{1D17A}', true, false),
            ('\u{1D17B}', false, false),
            ('\u{DFFFF}', false, false),
            ('\u{E0000}', true, false),
            ('\u{E007F}', true, false),
            ('\u{E0080}', false, false),
            ('\u{E00FF}', false, false),
            ('\u{E0100}', true, false),
            ('\u{E01EF}', true, false),
            ('\u{E01F0}', false, false),
        ];
        for &(c, invisible, bidi) in cases {
            assert_eq!(
                (is_invisible(c), is_bidi_control(c)),
                (invisible, bidi),
                "U+{:04X}",
                u32::from(c)
            );
        }
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::channel::{ChannelMetadataEdit, ChannelMetadataWrite};
    use chrono::{DateTime, Duration, Utc};
    use nostr::Keys;

    async fn setup() -> (Db, CommunityId) {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("landing-test-{}.example", id.simple()))
            .execute(&pool)
            .await
            .expect("insert test community");
        (Db::from_pool(pool), CommunityId::from_uuid(id))
    }

    fn key() -> Vec<u8> {
        Keys::generate().public_key().to_bytes().to_vec()
    }

    async fn channel(
        db: &Db,
        community: CommunityId,
        name: &str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
        created_by: &[u8],
        ttl_seconds: Option<i32>,
    ) -> Uuid {
        db.create_channel(
            community,
            name,
            channel_type,
            visibility,
            None,
            created_by,
            ttl_seconds,
        )
        .await
        .expect("create channel")
        .id
    }

    async fn open_stream(db: &Db, community: CommunityId, name: &str) -> Uuid {
        let creator = key();
        channel(
            db,
            community,
            name,
            ChannelType::Stream,
            ChannelVisibility::Open,
            &creator,
            None,
        )
        .await
    }

    /// Applies `edit` as a kind:9002 on `channel_id`, signed by a fresh key.
    async fn apply(
        db: &Db,
        community: CommunityId,
        channel_id: Uuid,
        edit: ChannelMetadataEdit,
    ) -> Result<ChannelMetadataWrite> {
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9002), "")
            .tags([nostr::Tag::parse(["h", &channel_id.to_string()]).expect("h tag")])
            .sign_with_keys(&Keys::generate())
            .expect("sign kind:9002");
        db.apply_channel_metadata_event(community, channel_id, &event, edit, &[])
            .await
    }

    fn designate() -> ChannelMetadataEdit {
        ChannelMetadataEdit {
            landing: true,
            ..Default::default()
        }
    }

    /// A `welcome` or `welcome_title` tag value: `""` restores the default.
    fn text(value: &str) -> Option<Option<String>> {
        Some((!value.is_empty()).then(|| value.to_string()))
    }

    /// The client's full-state form: `landing` plus `welcome` and
    /// `welcome_title`.
    fn full_state(welcome: &str, title: &str) -> ChannelMetadataEdit {
        ChannelMetadataEdit {
            landing: true,
            welcome: text(welcome),
            welcome_title: text(title),
            ..Default::default()
        }
    }

    fn refusal<T>(result: Result<T>) -> String {
        match result {
            Err(DbError::AccessDenied(reason)) => reason,
            Err(other) => panic!("expected a refusal, got {other:?}"),
            Ok(_) => panic!("expected a refusal, got success"),
        }
    }

    fn landing(channel_id: Uuid, welcome: Option<&str>, title: Option<&str>) -> CommunityLanding {
        CommunityLanding {
            channel_id: Some(channel_id),
            welcome: welcome.map(str::to_string),
            welcome_title: title.map(str::to_string),
        }
    }

    /// Seeds a relay member with `role`, added `seconds_ago`.
    async fn relay_member(
        db: &Db,
        community: CommunityId,
        role: &str,
        seconds_ago: i64,
    ) -> Vec<u8> {
        let pubkey = key();
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role, created_at, updated_at) \
             VALUES ($1, $2, $3, now() - make_interval(secs => $4), now())",
        )
        .bind(community.as_uuid())
        .bind(hex::encode(&pubkey))
        .bind(role)
        .bind(seconds_ago as f64)
        .execute(db.pool())
        .await
        .expect("insert relay member");
        pubkey
    }

    /// A kind:9002 `landing` designates the channel and moves the
    /// designation, carrying the welcome message and title; the full-state
    /// form on the current landing channel is idempotent; `welcome` and
    /// `welcome_title` are partial updates of each other; either elsewhere,
    /// and every ineligible post-state, is refused and changes nothing.
    ///
    /// Mutations: report `previous_landing` on a re-designation → RED;
    /// check eligibility on the pre-state → the same-event archive, private
    /// and ttl rows are accepted → RED; drop the welcome landing check →
    /// the welcome on `b` is accepted → RED; write `welcome_title`
    /// unconditionally in `set_welcome` → the welcome-alone edit clears the
    /// title → RED.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn kind_9002_moves_the_designation_and_accepts_the_full_state_form() {
        let (db, community) = setup().await;
        let a = open_stream(&db, community, "lobby").await;
        let b = open_stream(&db, community, "town-square").await;

        let write = apply(&db, community, a, full_state("Hi all", "Welcome aboard"))
            .await
            .expect("designate a");
        assert_eq!(write.previous_landing, None);
        assert_eq!(
            db.get_community_landing(community).await.expect("landing"),
            landing(a, Some("Hi all"), Some("Welcome aboard"))
        );

        let write = apply(&db, community, a, full_state("", ""))
            .await
            .expect("full state on the landing channel");
        assert!(write.inserted);
        assert_eq!(write.previous_landing, None, "re-designating is not a move");
        assert_eq!(
            db.get_community_landing(community).await.expect("landing"),
            landing(a, None, None),
            "empty values restore the defaults"
        );

        for edit in [
            ChannelMetadataEdit {
                welcome_title: text("Hello"),
                ..Default::default()
            },
            ChannelMetadataEdit {
                welcome: text("Second"),
                ..Default::default()
            },
        ] {
            apply(&db, community, a, edit)
                .await
                .expect("welcome or welcome_title alone on the landing channel");
        }
        assert_eq!(
            db.get_community_landing(community).await.expect("landing"),
            landing(a, Some("Second"), Some("Hello")),
            "each is a partial update of the other"
        );
        for edit in [
            ChannelMetadataEdit {
                welcome: text("Elsewhere"),
                ..Default::default()
            },
            ChannelMetadataEdit {
                welcome_title: text("Elsewhere"),
                ..Default::default()
            },
        ] {
            assert_eq!(
                refusal(apply(&db, community, b, edit).await),
                WELCOME_NOT_LANDING
            );
        }
        assert_eq!(
            db.get_community_landing(community).await.expect("landing"),
            landing(a, Some("Second"), Some("Hello"))
        );

        let write = apply(&db, community, b, designate())
            .await
            .expect("move to b");
        assert_eq!(write.previous_landing, Some(a));
        assert_eq!(
            db.get_community_landing(community).await.expect("landing"),
            landing(b, Some("Second"), Some("Hello")),
            "the welcome and its title move with the designation"
        );

        let private = channel(
            &db,
            community,
            "private-room",
            ChannelType::Stream,
            ChannelVisibility::Private,
            &key(),
            None,
        )
        .await;
        let ephemeral = channel(
            &db,
            community,
            "ephemeral-room",
            ChannelType::Stream,
            ChannelVisibility::Open,
            &key(),
            Some(3600),
        )
        .await;
        let ineligible = [
            (
                a,
                ChannelMetadataEdit {
                    archived: Some(true),
                    ..designate()
                },
                "archived by the same event",
            ),
            (
                a,
                ChannelMetadataEdit {
                    columns: ChannelUpdate {
                        visibility: Some("private".into()),
                        ..Default::default()
                    },
                    ..designate()
                },
                "made private by the same event",
            ),
            (
                a,
                ChannelMetadataEdit {
                    columns: ChannelUpdate {
                        ttl_seconds: Some(Some(3600)),
                        ..Default::default()
                    },
                    ..designate()
                },
                "given a ttl by the same event",
            ),
            (private, designate(), "a private channel"),
            (ephemeral, designate(), "a channel with a ttl"),
        ];
        for (target, edit, label) in ineligible {
            assert_eq!(
                refusal(apply(&db, community, target, edit).await),
                LANDING_INELIGIBLE,
                "{label}"
            );
        }
        assert_eq!(
            db.get_community_landing(community).await.expect("landing"),
            landing(b, Some("Second"), Some("Hello")),
            "a refused move changes nothing"
        );
        let a_after = db.get_channel(community, a).await.expect("a");
        assert!(
            a_after.archived_at.is_none(),
            "the refused archive rolled back"
        );
        assert_eq!(a_after.visibility, "open");
        assert_eq!(a_after.ttl_seconds, None);
    }

    /// The landing channel cannot be archived, made private, given a TTL or
    /// deleted on any path, while other edits stay allowed; after a move
    /// the previous landing channel can be.
    ///
    /// Mutations: drop the backstop in `set_archived_on`, `update_channel_on`
    /// or `soft_delete_on` → that row is accepted → RED.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn the_landing_channel_is_protected_on_every_path() {
        let (db, community) = setup().await;
        let lobby = open_stream(&db, community, "lobby").await;
        let other = open_stream(&db, community, "other").await;
        apply(&db, community, lobby, designate())
            .await
            .expect("designate");

        for (edit, label) in [
            (
                ChannelMetadataEdit {
                    archived: Some(true),
                    ..Default::default()
                },
                "kind:9002 archived",
            ),
            (
                ChannelMetadataEdit {
                    columns: ChannelUpdate {
                        visibility: Some("private".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                "kind:9002 visibility",
            ),
            (
                ChannelMetadataEdit {
                    columns: ChannelUpdate {
                        ttl_seconds: Some(Some(60)),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                "kind:9002 ttl",
            ),
        ] {
            assert_eq!(
                refusal(apply(&db, community, lobby, edit).await),
                LANDING_PROTECTED,
                "{label}"
            );
        }
        assert_eq!(
            refusal(db.archive_channel(community, lobby).await),
            LANDING_PROTECTED,
            "archive_channel"
        );
        assert_eq!(
            refusal(
                db.update_channel(
                    community,
                    lobby,
                    ChannelUpdate {
                        visibility: Some("private".into()),
                        ..Default::default()
                    },
                )
                .await
            ),
            LANDING_PROTECTED,
            "update_channel visibility"
        );
        assert_eq!(
            refusal(
                db.update_channel(
                    community,
                    lobby,
                    ChannelUpdate {
                        ttl_seconds: Some(Some(60)),
                        ..Default::default()
                    },
                )
                .await
            ),
            LANDING_PROTECTED,
            "update_channel ttl"
        );
        assert_eq!(
            refusal(db.soft_delete_channel(community, lobby).await),
            LANDING_PROTECTED,
            "soft_delete_channel"
        );

        db.update_channel(
            community,
            lobby,
            ChannelUpdate {
                name: Some("lobby-renamed".into()),
                unlisted: Some(true),
                auto_join: Some(true),
                ttl_seconds: Some(None),
                ..Default::default()
            },
        )
        .await
        .expect("rename, unlisted, auto_join and clearing a ttl stay allowed");
        let after = db.get_channel(community, lobby).await.expect("lobby");
        assert!(after.archived_at.is_none() && after.deleted_at.is_none());
        assert_eq!(after.visibility, "open");
        assert_eq!(after.ttl_seconds, None);

        apply(&db, community, other, designate())
            .await
            .expect("move away");
        db.archive_channel(community, lobby)
            .await
            .expect("the previous landing channel can be archived");
        assert!(db
            .soft_delete_channel(community, lobby)
            .await
            .expect("and deleted"));
    }

    struct Seed {
        name: &'static str,
        channel_type: ChannelType,
        visibility: ChannelVisibility,
        unlisted: bool,
        archived: bool,
        ttl_seconds: Option<i32>,
    }

    const fn general() -> Seed {
        Seed {
            name: "general",
            channel_type: ChannelType::Stream,
            visibility: ChannelVisibility::Open,
            unlisted: false,
            archived: false,
            ttl_seconds: None,
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum Expect {
        /// The seed at this index is adopted.
        Reused(usize),
        /// A new `general` is created, owned by the relay member at this
        /// index.
        Created(usize),
        NoSteward,
        NameTaken,
    }

    /// `ensure_landing_channel`'s rules: which `general` is adopted, when
    /// one is created and by whom it is owned, and when nothing is done.
    /// A second call returns the same designation.
    ///
    /// Mutations: drop `unlisted ASC` from the candidate order → "listed
    /// before an older unlisted" adopts the unlisted one → RED; adopt
    /// without the eligibility predicate → the archived, forum, ttl and
    /// private rows adopt them → RED; order the steward by `created_at`
    /// alone → "owner before an earlier admin" → RED; count an eligible
    /// holder in the name pre-read → "listed general" reads as held, which
    /// a concurrent ensure that lost the creation race would report as
    /// `SkippedNameTaken` → RED.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn ensure_landing_channel_rules() {
        let owner = || vec![("owner", 10)];
        let cases: Vec<(&str, Vec<Seed>, Vec<(&str, i64)>, Expect)> = vec![
            ("listed general", vec![general()], vec![], Expect::Reused(0)),
            (
                "name is case-insensitive",
                vec![Seed {
                    name: "General",
                    ..general()
                }],
                vec![],
                Expect::Reused(0),
            ),
            (
                "unlisted general is reused, not duplicated",
                vec![Seed {
                    unlisted: true,
                    ..general()
                }],
                owner(),
                Expect::Reused(0),
            ),
            (
                "listed before an older unlisted",
                vec![
                    Seed {
                        unlisted: true,
                        ..general()
                    },
                    general(),
                ],
                vec![],
                Expect::Reused(1),
            ),
            (
                "an archived general holds the name",
                vec![Seed {
                    archived: true,
                    ..general()
                }],
                owner(),
                Expect::NameTaken,
            ),
            (
                "a forum general holds the name",
                vec![Seed {
                    channel_type: ChannelType::Forum,
                    ..general()
                }],
                owner(),
                Expect::NameTaken,
            ),
            (
                "a general with a ttl holds the name",
                vec![Seed {
                    ttl_seconds: Some(3600),
                    ..general()
                }],
                owner(),
                Expect::NameTaken,
            ),
            (
                "a private general is left alone",
                vec![Seed {
                    visibility: ChannelVisibility::Private,
                    ..general()
                }],
                owner(),
                Expect::Created(0),
            ),
            (
                "other channels do not count",
                vec![Seed {
                    name: "random",
                    ..general()
                }],
                owner(),
                Expect::Created(0),
            ),
            ("no owner or admin", vec![], vec![], Expect::NoSteward),
            (
                "a member is not a steward",
                vec![],
                vec![("member", 10)],
                Expect::NoSteward,
            ),
            (
                "an admin stewards without an owner",
                vec![],
                vec![("member", 100), ("admin", 10)],
                Expect::Created(1),
            ),
            (
                "the owner before an earlier admin",
                vec![],
                vec![("admin", 100), ("owner", 10)],
                Expect::Created(1),
            ),
            (
                "the earliest owner",
                vec![],
                vec![("owner", 10), ("owner", 100)],
                Expect::Created(1),
            ),
        ];

        for (label, seeds, members, expect) in cases {
            let (db, community) = setup().await;
            let mut seeded = Vec::new();
            for seed in &seeds {
                let id = channel(
                    &db,
                    community,
                    seed.name,
                    seed.channel_type,
                    seed.visibility,
                    &key(),
                    seed.ttl_seconds,
                )
                .await;
                if seed.unlisted {
                    db.update_channel(
                        community,
                        id,
                        ChannelUpdate {
                            unlisted: Some(true),
                            ..Default::default()
                        },
                    )
                    .await
                    .expect("unlist seed");
                }
                if seed.archived {
                    db.archive_channel(community, id)
                        .await
                        .expect("archive seed");
                }
                seeded.push(id);
            }
            let mut stewards = Vec::new();
            for (role, seconds_ago) in members {
                stewards.push(relay_member(&db, community, role, seconds_ago).await);
            }

            assert_eq!(
                general_name_held_by_ineligible(db.pool(), community)
                    .await
                    .expect("name read"),
                matches!(expect, Expect::NameTaken),
                "{label}: only a channel that cannot be adopted holds the name"
            );
            let adoption = db
                .ensure_landing_channel(community)
                .await
                .unwrap_or_else(|e| panic!("{label}: {e:?}"));
            match (expect, adoption) {
                (Expect::Reused(index), LandingAdoption::Reused(id)) => {
                    assert_eq!(id, seeded[index], "{label}")
                }
                (
                    Expect::Created(index),
                    LandingAdoption::Created {
                        channel_id,
                        steward,
                    },
                ) => {
                    assert_eq!(steward.as_slice(), stewards[index].as_slice(), "{label}");
                    assert!(!seeded.contains(&channel_id), "{label}: a new channel");
                    let created = db
                        .get_channel(community, channel_id)
                        .await
                        .expect("created");
                    assert_eq!(created.name, "general", "{label}");
                    assert_eq!(
                        (created.channel_type.as_str(), created.visibility.as_str()),
                        ("stream", "open"),
                        "{label}"
                    );
                    assert!(!created.unlisted, "{label}: listed");
                    assert_eq!(created.created_by, stewards[index], "{label}");
                    assert_eq!(
                        db.get_member_role(community, channel_id, &stewards[index])
                            .await
                            .expect("steward role")
                            .as_deref(),
                        Some("owner"),
                        "{label}: the steward owns it"
                    );
                }
                (Expect::NoSteward, LandingAdoption::SkippedNoSteward)
                | (Expect::NameTaken, LandingAdoption::SkippedNameTaken) => {}
                (expect, got) => panic!("{label}: expected {expect:?}, got {got:?}"),
            }

            let designated = db
                .get_community_landing(community)
                .await
                .expect("landing")
                .channel_id;
            assert_eq!(designated, adoption.channel_id(), "{label}");
            if let Some(id) = designated {
                assert_eq!(
                    db.ensure_landing_channel(community).await.expect("again"),
                    LandingAdoption::Existing(id),
                    "{label}: a second call keeps it"
                );
            }
            for (seed, id) in seeds.iter().zip(&seeded) {
                let after = db.get_channel(community, *id).await.expect("seed");
                assert_eq!(
                    after.visibility,
                    seed.visibility.to_string(),
                    "{label}: seed untouched"
                );
            }
        }
    }

    /// Concurrent ensures on a community with no `general` create exactly
    /// one and agree on it.
    ///
    /// Mutation: drop both the locked designation re-check in
    /// `create_general` and the `ChannelNameTaken` fallback → a racing
    /// creator fails on the name index → RED.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_ensures_designate_one_general() {
        let (db, community) = setup().await;
        relay_member(&db, community, "owner", 10).await;
        let runs = (0..8).map(|_| {
            let db = db.clone();
            tokio::spawn(async move { db.ensure_landing_channel(community).await })
        });
        let mut adoptions = Vec::new();
        for run in runs.collect::<Vec<_>>() {
            adoptions.push(run.await.expect("join").expect("ensure"));
        }
        let ids: std::collections::HashSet<_> =
            adoptions.iter().map(LandingAdoption::channel_id).collect();
        assert_eq!(ids.len(), 1, "every caller agrees: {adoptions:?}");
        assert!(ids.iter().all(Option::is_some));
        assert_eq!(
            adoptions
                .iter()
                .filter(|a| matches!(a, LandingAdoption::Created { .. }))
                .count(),
            1,
            "{adoptions:?}"
        );
        let generals: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM channels WHERE community_id = $1 AND lower(name) = 'general'",
        )
        .bind(community.as_uuid())
        .fetch_one(db.pool())
        .await
        .expect("count generals");
        assert_eq!(generals, 1);
    }

    #[derive(Debug, Clone, Copy)]
    enum Shape {
        OpenStream,
        Forum,
        Dm,
        PrivateOwn,
        PrivateOther,
        DeletedStream,
    }

    /// Creates a channel of `shape` with a `person` membership row joined
    /// at `joined` and removed at `removed`.
    async fn membership(
        db: &Db,
        community: CommunityId,
        person: &[u8],
        shape: Shape,
        joined: DateTime<Utc>,
        removed: Option<DateTime<Utc>>,
    ) -> Uuid {
        let other = key();
        let (channel_type, visibility, creator) = match shape {
            Shape::OpenStream | Shape::DeletedStream => (
                ChannelType::Stream,
                ChannelVisibility::Open,
                other.as_slice(),
            ),
            Shape::Forum => (
                ChannelType::Forum,
                ChannelVisibility::Open,
                other.as_slice(),
            ),
            Shape::Dm => (
                ChannelType::Dm,
                ChannelVisibility::Private,
                other.as_slice(),
            ),
            Shape::PrivateOwn => (ChannelType::Stream, ChannelVisibility::Private, person),
            Shape::PrivateOther => (
                ChannelType::Stream,
                ChannelVisibility::Private,
                other.as_slice(),
            ),
        };
        let name = format!("evidence-{}", Uuid::new_v4().simple());
        let id = channel(
            db,
            community,
            &name,
            channel_type,
            visibility,
            creator,
            None,
        )
        .await;
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, joined_at, removed_at) \
             VALUES ($1, $2, $3, 'member', $4, $5) \
             ON CONFLICT (community_id, channel_id, pubkey) \
             DO UPDATE SET joined_at = EXCLUDED.joined_at, removed_at = EXCLUDED.removed_at",
        )
        .bind(community.as_uuid())
        .bind(id)
        .bind(person)
        .bind(joined)
        .bind(removed)
        .execute(db.pool())
        .await
        .expect("seed membership");
        if matches!(shape, Shape::DeletedStream) {
            sqlx::query(
                "UPDATE channels SET deleted_at = now() WHERE community_id = $1 AND id = $2",
            )
            .bind(community.as_uuid())
            .bind(id)
            .execute(db.pool())
            .await
            .expect("delete channel");
        }
        id
    }

    /// `has_entered_community`: which memberships are entry evidence, with
    /// a roster admission time (roster mode) and without (roster-free).
    ///
    /// Mutations: drop the channel-type filter → the DM row counts → RED;
    /// drop the own-private exclusion → the Solo row counts → RED; ignore
    /// `admitted_at` → "left before admission" counts in roster mode → RED.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn entry_evidence_rules() {
        let admitted = Utc::now() - Duration::hours(1);
        let (before, after) = (
            admitted - Duration::hours(1),
            admitted + Duration::minutes(10),
        );
        // (label, membership, roster-mode evidence, roster-free evidence)
        let cases: &[(
            &str,
            Option<(Shape, DateTime<Utc>, Option<DateTime<Utc>>)>,
            bool,
            bool,
        )] = &[
            ("no memberships", None, false, false),
            (
                "active, joined before admission",
                Some((Shape::OpenStream, before, None)),
                true,
                true,
            ),
            (
                "left before admission",
                Some((Shape::OpenStream, before, Some(before))),
                false,
                true,
            ),
            (
                "left after admission",
                Some((Shape::OpenStream, before, Some(after))),
                true,
                true,
            ),
            (
                "joined and left after admission",
                Some((Shape::OpenStream, after, Some(after))),
                true,
                true,
            ),
            (
                "forum member",
                Some((Shape::Forum, before, None)),
                true,
                true,
            ),
            ("DM only", Some((Shape::Dm, after, None)), false, false),
            (
                "own private chat (Solo)",
                Some((Shape::PrivateOwn, after, None)),
                false,
                false,
            ),
            (
                "someone else's private channel",
                Some((Shape::PrivateOther, after, None)),
                true,
                true,
            ),
            (
                "a deleted channel is still history",
                Some((Shape::DeletedStream, after, None)),
                true,
                true,
            ),
        ];
        for (label, seed, roster, roster_free) in cases {
            let (db, community) = setup().await;
            let person = key();
            if let Some((shape, joined, removed)) = seed {
                membership(&db, community, &person, *shape, *joined, *removed).await;
            }
            let entered = |admitted_at| {
                let db = db.clone();
                let person = person.clone();
                async move {
                    db.has_entered_community(community, &person, admitted_at)
                        .await
                        .expect("evidence")
                }
            };
            assert_eq!(
                entered(Some(admitted)).await,
                *roster,
                "{label}: roster mode"
            );
            assert_eq!(entered(None).await, *roster_free, "{label}: roster-free");
        }
    }

    /// `put_first_member_since` writes only when no row stands for this
    /// admission: none at all, or one removed before the roster admission.
    /// A leave since admission stands, and so does any earlier leave
    /// without an admission time.
    ///
    /// Mutation: drop the `removed_at >= $4` arm → "left after admission"
    /// is re-joined → RED; drop the `$4 IS NULL` arm → roster-free
    /// re-joins a leave → RED.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn put_first_member_since_rules() {
        let admitted = Utc::now() - Duration::hours(1);
        let (before, after) = (
            admitted - Duration::hours(1),
            admitted + Duration::minutes(10),
        );
        // (label, existing row (joined, removed), admitted_at, writes)
        let cases: &[(
            &str,
            Option<(DateTime<Utc>, Option<DateTime<Utc>>)>,
            Option<DateTime<Utc>>,
            bool,
        )] = &[
            ("no row, roster", None, Some(admitted), true),
            ("no row, roster-free", None, None, true),
            ("active row", Some((before, None)), Some(admitted), false),
            (
                "left before admission: re-admission restores",
                Some((before, Some(before))),
                Some(admitted),
                true,
            ),
            (
                "left after admission: the leave stands",
                Some((before, Some(after))),
                Some(admitted),
                false,
            ),
            (
                "left, roster-free: the leave stands",
                Some((before, Some(before))),
                None,
                false,
            ),
        ];
        for (label, row, admitted_at, writes) in cases {
            let (db, community) = setup().await;
            let person = key();
            let lobby = open_stream(&db, community, "lobby").await;
            if let Some((joined, removed)) = row {
                sqlx::query(
                    "INSERT INTO channel_members (community_id, channel_id, pubkey, role, joined_at, removed_at) \
                     VALUES ($1, $2, $3, 'member', $4, $5)",
                )
                .bind(community.as_uuid())
                .bind(lobby)
                .bind(&person)
                .bind(joined)
                .bind(removed)
                .execute(db.pool())
                .await
                .expect("seed row");
            }
            let wrote = db
                .put_first_member_since(community, lobby, &person, *admitted_at)
                .await
                .unwrap_or_else(|e| panic!("{label}: {e:?}"))
                .is_some();
            assert_eq!(wrote, *writes, "{label}");
            let active = db
                .get_member_role(community, lobby, &person)
                .await
                .expect("role")
                .is_some();
            assert_eq!(
                active,
                *writes || row.is_some_and(|(_, removed)| removed.is_none()),
                "{label}"
            );
        }
    }
}
