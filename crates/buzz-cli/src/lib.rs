mod client;
mod commands;
mod error;
mod validate;

use clap::{Parser, Subcommand};
use client::BuzzClient;
use error::CliError;
use nostr::Keys;

/// Run the Buzz CLI from raw arguments (including `argv[0]`).
///
/// Returns a process exit code (0 = success).
///
/// # Example
///
/// ```ignore
/// let code = buzz_cli::run_from_args(std::env::args()).await;
/// std::process::exit(code);
/// ```
pub async fn run_from_args<I, S>(args: I) -> i32
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            if e.use_stderr() {
                error::print_error(&CliError::Usage(e.to_string()));
                return 1;
            } else {
                // --help and --version: print normally (intentional human output)
                let _ = e.print();
                return 0;
            }
        }
    };
    match run(cli).await {
        Ok(()) => 0,
        Err(e) => {
            error::print_error(&e);
            error::exit_code(&e)
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "buzz",
    about = "Buzz CLI — interact with a Buzz relay",
    long_about = "\
Buzz CLI — interact with a Buzz relay

Configuration (flags override env vars):
  BUZZ_RELAY_URL     Relay base URL        [default: http://localhost:3000]
  BUZZ_PRIVATE_KEY   Nostr private key (hex or nsec)  [required]
  BUZZ_AUTH_TAG      NIP-OA auth tag JSON  [optional]

The 'pack' subcommand runs locally and does not require a relay connection.

Exit codes: 0=ok  1=bad input  2=relay/network error  3=auth error  4=other  5=write conflict
Errors are JSON on stderr: {\"error\": \"<category>\", \"message\": \"<detail>\"}"
)]
struct Cli {
    /// Relay URL (http:// or https://). Overrides BUZZ_RELAY_URL env var.
    #[arg(long, env = "BUZZ_RELAY_URL", default_value = "http://localhost:3000")]
    relay: String,

    /// Nostr private key (hex or nsec). This is the CLI's identity.
    #[arg(long, env = "BUZZ_PRIVATE_KEY")]
    private_key: Option<String>,

    /// NIP-OA auth tag JSON (owner attestation). Injected into every signed event.
    #[arg(long, env = "BUZZ_AUTH_TAG")]
    auth_tag: Option<String>,

    /// Output format: 'json' (default, full fields) or 'compact' (reduced fields).
    #[arg(long, value_enum, default_value = "json")]
    format: OutputFormat,

    #[command(subcommand)]
    command: Cmd,
}

// ---------------------------------------------------------------------------
// Value enums for typed --type / --visibility / --status flags
// ---------------------------------------------------------------------------

#[derive(Clone, clap::ValueEnum)]
pub enum ChannelType {
    #[value(name = "stream")]
    Stream,
    #[value(name = "forum")]
    Forum,
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stream => write!(f, "stream"),
            Self::Forum => write!(f, "forum"),
        }
    }
}

#[derive(Clone, clap::ValueEnum)]
pub enum ChannelVisibility {
    #[value(name = "open")]
    Open,
    #[value(name = "private")]
    Private,
}

impl std::fmt::Display for ChannelVisibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Private => write!(f, "private"),
        }
    }
}

#[derive(Clone, clap::ValueEnum)]
pub enum PresenceStatus {
    #[value(name = "online")]
    Online,
    #[value(name = "away")]
    Away,
    #[value(name = "offline")]
    Offline,
}

#[derive(Clone, clap::ValueEnum)]
pub enum EmojiScope {
    #[value(name = "own")]
    Own,
    #[value(name = "workspace")]
    Workspace,
}

impl std::fmt::Display for PresenceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Online => write!(f, "online"),
            Self::Away => write!(f, "away"),
            Self::Offline => write!(f, "offline"),
        }
    }
}

/// Output format for read commands.
#[derive(Clone, clap::ValueEnum, Default)]
pub enum OutputFormat {
    /// Full normalized JSON (default)
    #[default]
    #[value(name = "json")]
    Json,
    /// Reduced fields for agent scanning
    #[value(name = "compact")]
    Compact,
}

// ---------------------------------------------------------------------------
// Subcommand groups
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
enum Cmd {
    /// Send, read, search, and manage messages
    #[command(subcommand)]
    Messages(MessagesCmd),
    /// Create, configure, and manage channels
    #[command(subcommand)]
    Channels(ChannelsCmd),
    /// Get and set channel canvas documents
    #[command(subcommand)]
    Canvas(CanvasCmd),
    /// Add, remove, and list emoji reactions
    #[command(subcommand)]
    Reactions(ReactionsCmd),
    /// Manage your custom emoji set (workspace palette is the union of all members' sets)
    #[command(subcommand)]
    Emoji(EmojiCmd),
    /// List, open, and manage direct messages
    #[command(subcommand)]
    Dms(DmsCmd),
    /// Look up users and manage profiles and presence
    #[command(subcommand)]
    Users(UsersCmd),
    /// Create, trigger, and manage workflows
    #[command(subcommand)]
    Workflows(WorkflowsCmd),
    /// Read the activity feed
    #[command(subcommand)]
    Feed(FeedCmd),
    /// Publish notes and manage the social graph (NIP-01/02)
    #[command(subcommand)]
    Social(SocialCmd),
    /// Publish and edit long-form NIP-23 notes — team knowledge base
    #[command(subcommand)]
    Notes(NotesCmd),
    /// Announce and discover git repositories (NIP-34)
    #[command(subcommand)]
    Repos(ReposCmd),
    /// Send, get, list, and set status on git patches (NIP-34)
    #[command(subcommand)]
    Patches(PatchesCmd),
    /// Create, get, list, and set status on git issues (NIP-34)
    #[command(subcommand)]
    Issues(IssuesCmd),
    /// Upload files to the relay's Blossom store
    #[command(subcommand)]
    Upload(UploadCmd),
    /// Agent engram management — persistent memory per NIP-AE
    #[command(subcommand)]
    Mem(MemCmd),
    /// Persona pack operations (local, no relay connection needed)
    #[command(subcommand)]
    Pack(PackCmd),
}

// ---------------------------------------------------------------------------
// Messages subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum MessagesCmd {
    /// Send a message to a channel
    #[command(
        after_help = "Examples:\n  buzz messages send --channel <UUID> --content \"hello\"\n  buzz messages send --channel <UUID> --content \"@alice check this\"\n  echo \"hello from stdin\" | buzz messages send --channel <UUID> --content -"
    )]
    Send {
        /// Channel UUID (from 'buzz channels list')
        #[arg(long)]
        channel: String,
        /// Message text — supports @mentions and markdown. Use '-' to read from stdin.
        #[arg(long)]
        content: String,
        /// Nostr event kind (default: channel default)
        #[arg(long)]
        kind: Option<u16>,
        /// Event ID to reply to (creates a thread)
        #[arg(long)]
        reply_to: Option<String>,
        /// Also publish to the Nostr network
        #[arg(long, default_value_t = false)]
        broadcast: bool,
        /// Attach file(s) — uploads and includes as imeta tags
        #[arg(long = "file")]
        files: Vec<String>,
    },
    /// Send a code diff / patch to a channel
    SendDiff {
        /// Channel UUID
        #[arg(long)]
        channel: String,
        /// Diff/patch content (use '-' to read from stdin)
        #[arg(long)]
        diff: String,
        /// Repository URL (e.g. https://github.com/org/repo)
        #[arg(long)]
        repo: String,
        /// Commit SHA
        #[arg(long)]
        commit: String,
        /// Single file path within the repo
        #[arg(long)]
        file: Option<String>,
        /// Parent commit SHA for three-way diff context
        #[arg(long)]
        parent_commit: Option<String>,
        /// Source branch name
        #[arg(long)]
        source_branch: Option<String>,
        /// Target branch name
        #[arg(long)]
        target_branch: Option<String>,
        /// Pull request number
        #[arg(long)]
        pr: Option<u32>,
        /// Language hint (auto-detected from file extension if omitted)
        #[arg(long)]
        lang: Option<String>,
        /// Human-readable description of the change
        #[arg(long)]
        description: Option<String>,
        /// Event ID to reply to (creates a thread)
        #[arg(long)]
        reply_to: Option<String>,
    },
    /// Edit a previously sent message
    Edit {
        /// Event ID of the message to edit (64-char hex)
        #[arg(long)]
        event: String,
        /// New message content
        #[arg(long)]
        content: String,
    },
    /// Delete a message by event ID
    Delete {
        /// Event ID to delete (64-char hex)
        #[arg(long)]
        event: String,
    },
    /// Retrieve messages from a channel
    #[command(
        after_help = "Examples:\n  buzz messages get --channel <UUID>\n  buzz messages get --channel <UUID> --limit 50 --kinds 1,1984"
    )]
    Get {
        /// Channel UUID
        #[arg(long)]
        channel: String,
        /// Maximum number of results to return
        #[arg(long)]
        limit: Option<u32>,
        /// Unix timestamp — return messages before this time
        #[arg(long)]
        before: Option<i64>,
        /// Unix timestamp — return messages after this time
        #[arg(long)]
        since: Option<i64>,
        /// Comma-separated event kinds to filter (e.g. 1,1984)
        #[arg(long)]
        kinds: Option<String>,
    },
    /// Get a message thread (replies to a root message)
    Thread {
        /// Channel UUID
        #[arg(long)]
        channel: String,
        /// Root message event ID (64-char hex)
        #[arg(long)]
        event: String,
        /// Maximum number of results to return
        #[arg(long)]
        limit: Option<u32>,
        /// Maximum reply nesting depth to include
        #[arg(long)]
        depth_limit: Option<u32>,
    },
    /// Full-text search across messages
    Search {
        /// Search query string
        #[arg(long)]
        query: String,
        /// Maximum number of results to return
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Upvote or downvote a forum post
    Vote {
        /// Event ID of the post to vote on (64-char hex)
        #[arg(long)]
        event: String,
        /// Vote direction: "up" or "down"
        #[arg(long)]
        direction: String,
    },
}

// ---------------------------------------------------------------------------
// Channels subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum ChannelsCmd {
    /// List channels visible to the current identity
    #[command(
        after_help = "Examples:\n  buzz channels list\n  buzz channels list --visibility open"
    )]
    List {
        /// Filter by visibility
        #[arg(long, value_enum)]
        visibility: Option<ChannelVisibility>,
        /// Only show channels where the current identity is a member
        #[arg(long, default_value_t = false)]
        member: bool,
        /// Maximum number of channels to return [default: 500]
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Get details for a single channel
    Get {
        /// Channel UUID
        #[arg(long)]
        channel: String,
    },
    /// Search channels by human-readable name
    #[command(
        after_help = "Examples:\n  buzz channels search --query composer\n  buzz channels search --query buzz-chat-composer --exact\n  buzz channels search --query design --include-archived"
    )]
    Search {
        /// Search query (case-insensitive substring of channel name)
        #[arg(long)]
        query: String,
        /// Require an exact case-insensitive match instead of substring
        #[arg(long, default_value_t = false)]
        exact: bool,
        /// Include archived channels in results
        #[arg(long, default_value_t = false)]
        include_archived: bool,
        /// Maximum number of channel-metadata events to fetch from the relay
        #[arg(long, default_value_t = 1000)]
        limit: u32,
    },
    /// Create a new channel
    #[command(
        after_help = "Examples:\n  buzz channels create --name general --type stream --visibility open\n  buzz channels create --name design --type forum --visibility open --description \"Design discussions\"\n  buzz channels create --name standup --type stream --visibility open --ttl 3600  # ephemeral, archived after 1h idle"
    )]
    Create {
        /// Channel name
        #[arg(long)]
        name: String,
        /// Channel type
        #[arg(long = "type", value_enum)]
        channel_type: ChannelType,
        /// Channel visibility
        #[arg(long, value_enum)]
        visibility: ChannelVisibility,
        /// Channel description
        #[arg(long)]
        description: Option<String>,
        /// Make the channel ephemeral: lifetime in seconds. The relay archives
        /// it once this many seconds pass without a new message.
        #[arg(long, value_name = "SECONDS")]
        ttl: Option<i64>,
    },
    /// Update channel name, description, or ephemeral TTL
    Update {
        /// Channel UUID
        #[arg(long)]
        channel: String,
        /// New channel name
        #[arg(long)]
        name: Option<String>,
        /// New channel description
        #[arg(long)]
        description: Option<String>,
        /// Make the channel ephemeral (or change its lifetime): seconds until
        /// the relay archives it after the last message. Conflicts with --no-ttl.
        #[arg(long, value_name = "SECONDS", conflicts_with = "no_ttl")]
        ttl: Option<i64>,
        /// Clear an existing TTL, making the channel permanent.
        #[arg(long)]
        no_ttl: bool,
    },
    /// Set the channel topic
    Topic {
        /// Channel UUID
        #[arg(long)]
        channel: String,
        /// New topic text
        #[arg(long)]
        topic: String,
    },
    /// Set the channel purpose
    Purpose {
        /// Channel UUID
        #[arg(long)]
        channel: String,
        /// New purpose text
        #[arg(long)]
        purpose: String,
    },
    /// Join a channel
    Join {
        /// Channel UUID
        #[arg(long)]
        channel: String,
    },
    /// Leave a channel
    Leave {
        /// Channel UUID
        #[arg(long)]
        channel: String,
    },
    /// Archive a channel
    Archive {
        /// Channel UUID
        #[arg(long)]
        channel: String,
    },
    /// Unarchive a channel
    Unarchive {
        /// Channel UUID
        #[arg(long)]
        channel: String,
    },
    /// Delete a channel permanently
    Delete {
        /// Channel UUID
        #[arg(long)]
        channel: String,
    },
    /// List members of a channel
    Members {
        /// Channel UUID
        #[arg(long)]
        channel: String,
    },
    /// Add a member to a channel
    #[command(name = "add-member")]
    AddMember {
        /// Channel UUID
        #[arg(long)]
        channel: String,
        /// Member pubkey (64-char hex)
        #[arg(long)]
        pubkey: String,
        /// Member role (owner, admin, member, guest, bot)
        #[arg(long)]
        role: Option<String>,
    },
    /// Remove a member from a channel
    #[command(name = "remove-member")]
    RemoveMember {
        /// Channel UUID
        #[arg(long)]
        channel: String,
        /// Member pubkey (64-char hex)
        #[arg(long)]
        pubkey: String,
    },
    /// Set your channel addition policy
    #[command(name = "set-add-policy")]
    SetAddPolicy {
        /// Policy: anyone | owner_only | nobody
        #[arg(long)]
        policy: String,
    },
}

// ---------------------------------------------------------------------------
// Canvas subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum CanvasCmd {
    /// Get the canvas document for a channel
    Get {
        /// Channel UUID
        #[arg(long)]
        channel: String,
    },
    /// Set (replace) the canvas document for a channel
    Set {
        /// Channel UUID
        #[arg(long)]
        channel: String,
        /// Canvas content (markdown; use '-' to read from stdin)
        #[arg(long)]
        content: String,
    },
}

// ---------------------------------------------------------------------------
// Reactions subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum ReactionsCmd {
    /// Add an emoji reaction to a message
    Add {
        /// Event ID (64-char hex)
        #[arg(long)]
        event: String,
        /// Emoji character (e.g. '👍') or custom emoji shortcode
        #[arg(long)]
        emoji: String,
        /// Image URL for a custom emoji reaction; when set, content becomes `:shortcode:`
        #[arg(long = "emoji-url")]
        emoji_url: Option<String>,
    },
    /// Remove an emoji reaction from a message
    Remove {
        /// Event ID (64-char hex)
        #[arg(long)]
        event: String,
        /// Emoji character to remove
        #[arg(long)]
        emoji: String,
    },
    /// List reactions on a message
    Get {
        /// Event ID (64-char hex)
        #[arg(long)]
        event: String,
    },
}

// ---------------------------------------------------------------------------
// Custom emoji subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum EmojiCmd {
    /// List the workspace custom emoji palette (union of every member's set)
    List,
    /// Add or update a custom emoji in your own set
    Set {
        /// Emoji shortcode, without surrounding colons
        #[arg(long)]
        shortcode: String,
        /// Image URL for the emoji
        #[arg(long)]
        url: String,
    },
    /// Remove a custom emoji from your own set
    Rm {
        /// Emoji shortcode, without surrounding colons
        #[arg(long)]
        shortcode: String,
    },
    /// Export custom emojis to stdout or a file
    Export {
        /// Write JSON to this file path instead of stdout
        #[arg(long)]
        file: Option<String>,
        /// Export your own set (default) or the full workspace palette
        #[arg(long, value_enum, default_value = "own")]
        scope: EmojiScope,
    },
    /// Import custom emojis from stdin or a file into your own set
    Import {
        /// Read JSON from this file path instead of stdin
        #[arg(long)]
        file: Option<String>,
        /// Replace your entire set instead of merging
        #[arg(long, default_value_t = false)]
        replace: bool,
        /// Print what would be published without writing
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

// ---------------------------------------------------------------------------
// DMs subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum DmsCmd {
    /// List direct message conversations
    List {
        /// Maximum number of results to return
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Open a new direct message with one or more users
    Open {
        /// User pubkey(s) to DM (64-char hex, 1-8)
        #[arg(long = "pubkey")]
        pubkeys: Vec<String>,
    },
    /// Add a member to an existing DM conversation
    AddMember {
        /// DM conversation UUID
        #[arg(long)]
        channel: String,
        /// User pubkey to add (64-char hex)
        #[arg(long)]
        pubkey: String,
    },
    /// Hide a DM conversation from your DM list
    Hide {
        /// DM conversation UUID
        #[arg(long)]
        channel: String,
    },
}

// ---------------------------------------------------------------------------
// Users subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum UsersCmd {
    /// Look up user profiles by pubkey or name
    Get {
        /// User pubkey(s) to look up (64-char hex). Omit for your own profile
        #[arg(long = "pubkey")]
        pubkeys: Vec<String>,
        /// Search by display name (case-insensitive substring match)
        #[arg(long = "name")]
        name: Option<String>,
    },
    /// Update the current identity's profile
    #[command(name = "set-profile")]
    SetProfile {
        /// Display name
        #[arg(long)]
        name: Option<String>,
        /// Avatar URL
        #[arg(long)]
        avatar: Option<String>,
        /// Bio / about text
        #[arg(long)]
        about: Option<String>,
        /// NIP-05 identifier (e.g. user@example.com)
        #[arg(long)]
        nip05: Option<String>,
    },
    /// Get presence status for users
    Presence {
        /// Comma-separated pubkeys (64-char hex)
        #[arg(long)]
        pubkeys: String,
    },
    /// Set your presence status (online/away/offline)
    #[command(name = "set-presence")]
    SetPresence {
        /// Presence status
        #[arg(long, value_enum)]
        status: PresenceStatus,
    },
}

// ---------------------------------------------------------------------------
// Workflows subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum WorkflowsCmd {
    /// List workflows in a channel
    List {
        /// Channel UUID
        #[arg(long)]
        channel: String,
    },
    /// Get details for a single workflow
    Get {
        /// Workflow UUID
        #[arg(long)]
        workflow: String,
    },
    /// Create a workflow from a YAML definition
    Create {
        /// Channel UUID
        #[arg(long)]
        channel: String,
        /// Workflow YAML definition
        #[arg(long)]
        yaml: String,
    },
    /// Update a workflow's YAML definition
    Update {
        /// Channel UUID the workflow belongs to
        #[arg(long)]
        channel: String,
        /// Workflow UUID
        #[arg(long)]
        workflow: String,
        /// Updated workflow YAML definition
        #[arg(long)]
        yaml: String,
    },
    /// Delete a workflow
    Delete {
        /// Workflow UUID
        #[arg(long)]
        workflow: String,
    },
    /// Trigger a workflow run
    #[command(
        after_help = "Examples:\n  buzz workflows trigger --workflow <UUID>\n  buzz workflows trigger --workflow <UUID> --inputs '{\"key\":\"value\"}'"
    )]
    Trigger {
        /// Workflow UUID
        #[arg(long)]
        workflow: String,
        /// JSON object of input variables passed to the workflow as event content
        #[arg(long)]
        inputs: Option<String>,
    },
    /// List runs for a workflow
    Runs {
        /// Workflow UUID
        #[arg(long)]
        workflow: String,
        /// Maximum number of results to return
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Approve or deny a workflow step
    #[command(
        after_help = "Examples:\n  buzz workflows approve --token <UUID>\n  buzz workflows approve --token <UUID> --approved false --note \"needs revision\""
    )]
    Approve {
        /// The approval token UUID (from the approval request)
        #[arg(long)]
        token: String,
        /// Approve (true) or deny (false) the step
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        approved: bool,
        /// Optional note to include with the approval/denial
        #[arg(long)]
        note: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Feed subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum FeedCmd {
    /// Get recent activity feed entries
    Get {
        /// Unix timestamp — return entries after this time
        #[arg(long)]
        since: Option<i64>,
        /// Maximum number of results to return
        #[arg(long)]
        limit: Option<u32>,
        /// Comma-separated feed types to include: mentions, needs_action, activity, agent_activity
        #[arg(long)]
        types: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Social subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum SocialCmd {
    /// Publish a text note (NIP-01 kind:1)
    #[command(name = "publish")]
    PublishNote {
        /// Text content of the note.
        #[arg(long)]
        content: String,
        /// 64-char hex event ID to reply to.
        #[arg(long)]
        reply_to: Option<String>,
    },
    /// Set your contact list (NIP-02 kind:3)
    #[command(name = "set-contacts")]
    SetContactList {
        /// JSON array of contacts: [{"pubkey":"hex","relay_url":"...","petname":"..."}]
        #[arg(long)]
        contacts: String,
    },
    /// Get a single event by ID
    #[command(name = "event")]
    GetEvent {
        /// 64-char hex event ID.
        #[arg(long)]
        event: String,
    },
    /// Get recent notes published by a user
    #[command(name = "notes")]
    GetUserNotes {
        /// 64-char hex pubkey of the author.
        #[arg(long)]
        pubkey: String,
        /// Maximum number of notes to return (default 50, max 100).
        #[arg(long)]
        limit: Option<u32>,
        /// Unix timestamp cursor — return notes created before this time.
        #[arg(long)]
        before: Option<i64>,
        /// Event ID cursor — return notes created before this event (composite pagination with --before).
        #[arg(long)]
        before_id: Option<String>,
    },
    /// Get a user's contact list
    #[command(name = "contacts")]
    GetContactList {
        /// 64-char hex pubkey.
        #[arg(long)]
        pubkey: String,
    },
    /// Publish a NIP-51/NIP-65 social list or set.
    #[command(name = "set-list")]
    SetList {
        /// Supported kind: 10000, 10001, 10002, 10003, 30000, or 30003.
        #[arg(long)]
        kind: u16,
        /// JSON array of Nostr tags, e.g. [["p","<hex>"],["d","friends"]].
        #[arg(long)]
        tags: String,
        /// Event content.
        #[arg(long, default_value = "")]
        content: String,
    },
    /// Get NIP-51/NIP-65 social lists or sets by author and kind.
    #[command(name = "list")]
    GetList {
        /// 64-char hex pubkey of the author.
        #[arg(long)]
        pubkey: String,
        /// Supported kind: 10000, 10001, 10002, 10003, 30000, or 30003.
        #[arg(long)]
        kind: u32,
        /// Optional d-tag for parameterized replaceable sets.
        #[arg(long)]
        d_tag: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Notes subcommands (NIP-23 long-form)
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum NotesCmd {
    /// Create or update a note. Idempotent upsert keyed by `(me, --name)`.
    ///
    /// `published_at` is preserved on edits (only set on first create).
    /// `--title` is required on first create; on subsequent edits the existing
    /// title is carried forward when `--title` is omitted, and `--title ""`
    /// explicitly clears it.
    #[command(
        after_help = "Examples:\n  echo '# Hello' | buzz notes set --name hello --title 'Hello' --content -\n  buzz notes set --name hello --tag onboarding --content - < draft.md"
    )]
    Set {
        /// Slug — becomes the `d` tag. `[a-z0-9._-]{1,80}`.
        #[arg(long)]
        name: String,
        /// Note title (NIP-23 `title` tag). Required on first create; omit to carry; `""` to clear.
        #[arg(long)]
        title: Option<String>,
        /// Short summary (NIP-23 `summary` tag). Omit to carry; `""` to clear.
        #[arg(long)]
        summary: Option<String>,
        /// Topic tag (NIP-23 `t` tag). May be repeated. Replaces (not merges) existing tags on edit; omit to carry forward.
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Clear all `t` tags on update. Mutually exclusive with `--tag`.
        /// Without this and without `--tag`, existing tags are carried forward.
        #[arg(long, default_value_t = false)]
        clear_tags: bool,
        /// Markdown body. Use `-` to read from stdin.
        #[arg(long)]
        content: String,
        /// Allow committing an empty body (refused by default to catch upstream pipeline failures).
        #[arg(long, default_value_t = false)]
        allow_empty: bool,
    },
    /// Read a note by `--naddr` (exact) or `--name <slug>` (cross-author lookup).
    Get {
        /// NIP-19 `naddr1…` or `30023:<pubkey>:<slug>` coordinate. Mutually exclusive with `--name`.
        #[arg(long)]
        naddr: Option<String>,
        /// Slug to look up across authors. Mutually exclusive with `--naddr`.
        #[arg(long)]
        name: Option<String>,
        /// Disambiguate `--name` to a specific author (hex pubkey, display name, or `me`).
        #[arg(long)]
        author: Option<String>,
        /// On an ambiguous `--name` (multiple authors), pick the most recently updated note
        /// instead of erroring. Mutually exclusive with `--author` and `--naddr`.
        #[arg(long, default_value_t = false)]
        latest: bool,
        /// Print only the markdown body, not the full event JSON.
        #[arg(long, default_value_t = false)]
        content_only: bool,
    },
    /// List notes. Defaults to your own.
    Ls {
        /// Hex pubkey, display name, `me`, or `all`.
        #[arg(long, default_value = "me")]
        author: Option<String>,
        /// Filter by NIP-23 `t` tag.
        #[arg(long)]
        tag: Option<String>,
        /// Max results (default 50, hard cap 200).
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Delete one of your own notes via NIP-09 (kind:5).
    ///
    /// Emits an a-tag-only deletion targeting the addressable coordinate
    /// `30023:<pubkey>:<slug>` (no `e` tag — an `e` tag would route around the
    /// relay's coordinate soft-delete and leave the note alive). Read-before-
    /// write gives a clean NotFound when there's nothing to delete.
    Rm {
        /// Slug of the note to delete. Only your own notes can be removed.
        #[arg(long)]
        name: String,
    },
}

// ---------------------------------------------------------------------------
// Repos subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum ReposCmd {
    /// Announce a git repository (NIP-34)
    Create {
        /// Repository identifier: [a-zA-Z0-9._-]{1,64}
        #[arg(long)]
        id: String,
        /// Human-readable display name
        #[arg(long)]
        name: Option<String>,
        /// Repository description
        #[arg(long)]
        description: Option<String>,
        /// Clone URL(s) — can be specified multiple times
        #[arg(long = "clone")]
        clone_urls: Vec<String>,
        /// Web browsing URL
        #[arg(long)]
        web: Option<String>,
        /// Preferred Nostr relay(s) for repo discovery — can be specified multiple times
        #[arg(long = "nostr-relay")]
        relays: Vec<String>,
    },
    /// Get a repository announcement
    Get {
        /// Repository identifier (d-tag)
        #[arg(long)]
        id: String,
        /// Owner pubkey (64-char hex). Omit to match any owner.
        #[arg(long)]
        owner: Option<String>,
    },
    /// List repository announcements
    List {
        /// Owner pubkey (64-char hex). Omit for your repos.
        #[arg(long)]
        owner: Option<String>,
        /// Maximum number of results
        #[arg(long)]
        limit: Option<u32>,
    },
}

// ---------------------------------------------------------------------------
// Patches subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum PatchesCmd {
    /// Send a git patch (NIP-34 kind:1617)
    #[command(
        after_help = "Examples:\n  git format-patch -1 HEAD --stdout | buzz patches send --repo-owner <hex> --repo-id myrepo --patch-file - --root\n  buzz patches send --repo-owner <hex> --repo-id myrepo --patch-file 0001-fix.patch --reply-to <prev-patch-id>"
    )]
    Send {
        /// Repo owner pubkey (64-char hex)
        #[arg(long)]
        repo_owner: String,
        /// Repo identifier (d-tag)
        #[arg(long)]
        repo_id: String,
        /// Path to a `git format-patch` file, or '-' to read from stdin
        #[arg(long)]
        patch_file: String,
        /// Earliest-unique-commit of the repo
        #[arg(long)]
        euc: Option<String>,
        /// Additional recipient pubkey(s) — can be specified multiple times
        #[arg(long = "to")]
        to: Vec<String>,
        /// Previous patch event id (series) or original root (revision)
        #[arg(long)]
        reply_to: Option<String>,
        /// Mark as the first patch of a new series
        #[arg(long, default_value_t = false)]
        root: bool,
        /// Mark as the first patch of a new revision of an existing series
        #[arg(long, default_value_t = false)]
        root_revision: bool,
        /// Commit ID this patch produces when applied
        #[arg(long)]
        commit: Option<String>,
        /// Parent commit ID
        #[arg(long)]
        parent_commit: Option<String>,
        /// PGP signature of the commit
        #[arg(long)]
        commit_pgp_sig: Option<String>,
        /// Committer identity: 'name|email|timestamp|tz-offset-minutes'
        #[arg(long)]
        committer: Option<String>,
    },
    /// Get a patch by event id
    Get {
        /// Patch event id (64-char hex)
        #[arg(long)]
        event: String,
    },
    /// List patches for a repo
    List {
        /// Repo owner pubkey (64-char hex)
        #[arg(long)]
        repo_owner: String,
        /// Repo identifier (d-tag)
        #[arg(long)]
        repo_id: String,
        /// Filter by patch author pubkey
        #[arg(long)]
        author: Option<String>,
        /// Maximum number of results
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Set status on a patch (open/merged/closed/draft — NIP-34 kind:1630-1633)
    Status {
        /// Root patch event id (first patch of the series/revision)
        #[arg(long)]
        root: String,
        /// New status
        #[arg(long, value_parser = ["open", "merged", "closed", "draft"])]
        status: String,
        /// Markdown context for the status change ('-' to read from stdin)
        #[arg(long)]
        content: Option<String>,
        /// Repo owner pubkey — requires --repo-id
        #[arg(long, requires = "repo_id")]
        repo_owner: Option<String>,
        /// Repo identifier (d-tag) — requires --repo-owner
        #[arg(long, requires = "repo_owner")]
        repo_id: Option<String>,
        /// Earliest-unique-commit of the repo
        #[arg(long)]
        euc: Option<String>,
        /// Root id of the revision that was accepted (status=merged only)
        #[arg(long)]
        revision: Option<String>,
        /// Additional recipient pubkey(s) for the status event (besides the
        /// repo owner, which is tagged automatically when --repo-owner is
        /// given) — e.g. root/revision author. Can be specified multiple times.
        #[arg(long = "to")]
        to: Vec<String>,
        /// Applied patch event id — can be specified multiple times (status=merged only).
        /// Accepts `<id>`, `<id>:<relay-url>`, or `<id>:<relay-url>:<pubkey>`.
        #[arg(long = "q")]
        q: Vec<String>,
        /// Merge commit id (status=merged only)
        #[arg(long)]
        merge_commit: Option<String>,
        /// Commit id applied to the target branch — can be specified multiple times (status=merged only)
        #[arg(long = "applied-as-commit")]
        applied_as_commit: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// Issues subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum IssuesCmd {
    /// Create a git issue (NIP-34 kind:1621)
    Create {
        /// Repo owner pubkey (64-char hex)
        #[arg(long)]
        repo_owner: String,
        /// Repo identifier (d-tag)
        #[arg(long)]
        repo_id: String,
        /// Issue title
        #[arg(long, alias = "subject")]
        title: String,
        /// Issue body, markdown. Use '-' to read from stdin.
        #[arg(long)]
        content: String,
        /// Label — can be specified multiple times
        #[arg(long = "label")]
        label: Vec<String>,
        /// Additional recipient pubkey(s) — can be specified multiple times
        #[arg(long = "to")]
        to: Vec<String>,
    },
    /// Get an issue by event id
    Get {
        /// Issue event id (64-char hex)
        #[arg(long)]
        event: String,
    },
    /// List issues for a repo
    List {
        /// Repo owner pubkey (64-char hex)
        #[arg(long)]
        repo_owner: String,
        /// Repo identifier (d-tag)
        #[arg(long)]
        repo_id: String,
        /// Filter by issue author pubkey
        #[arg(long)]
        author: Option<String>,
        /// Filter by label
        #[arg(long)]
        label: Option<String>,
        /// Maximum number of results
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Set status on an issue (open/resolved/closed/draft — NIP-34 kind:1630-1633)
    Status {
        /// Issue event id
        #[arg(long)]
        issue: String,
        /// New status
        #[arg(long, value_parser = ["open", "resolved", "closed", "draft"])]
        status: String,
        /// Markdown context for the status change ('-' to read from stdin)
        #[arg(long)]
        content: Option<String>,
        /// Repo owner pubkey — requires --repo-id
        #[arg(long, requires = "repo_id")]
        repo_owner: Option<String>,
        /// Repo identifier (d-tag) — requires --repo-owner
        #[arg(long, requires = "repo_owner")]
        repo_id: Option<String>,
        /// Earliest-unique-commit of the repo
        #[arg(long)]
        euc: Option<String>,
        /// Additional recipient pubkey(s) for the status event (besides the
        /// repo owner, which is tagged automatically when --repo-owner is
        /// given) — e.g. the issue author. Can be specified multiple times.
        #[arg(long = "to")]
        to: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// Upload subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
pub enum UploadCmd {
    /// Upload a file to the relay's Blossom store
    File {
        /// Path to the file to upload
        #[arg(long)]
        file: String,
    },
}

// ---------------------------------------------------------------------------
// Mem subcommands (NIP-AE)
// ---------------------------------------------------------------------------

/// Subcommands for `buzz mem`.
#[derive(Subcommand)]
pub enum MemCmd {
    /// List non-tombstoned memory entries
    Ls {
        /// Owner pubkey (hex). Overrides BUZZ_AUTH_TAG.
        #[arg(long)]
        owner: Option<String>,
        /// Agent pubkey (hex) to read as this key's owner.
        #[arg(long)]
        agent: Option<String>,
        /// Emit JSON instead of tab-delimited lines.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Print the value of a slug to stdout (no trailing newline)
    Get {
        slug: String,
        #[arg(long)]
        owner: Option<String>,
        /// Agent pubkey (hex) to read as this key's owner.
        #[arg(long)]
        agent: Option<String>,
    },
    /// Print sha256(value) in hex (use as `--base-hash` for `mem patch`).
    Hash {
        slug: String,
        #[arg(long)]
        owner: Option<String>,
        /// Agent pubkey (hex) to read as this key's owner.
        #[arg(long)]
        agent: Option<String>,
    },
    /// Set a slug's value. Pass `-` to read the value from stdin.
    Set {
        slug: String,
        value: String,
        #[arg(long)]
        owner: Option<String>,
        /// Allow committing an empty value. Without this, a zero-byte stdin
        /// read is rejected to prevent silent data loss from upstream
        /// pipeline failures.
        #[arg(long, default_value_t = false)]
        allow_empty: bool,
    },
    /// Apply a unified diff to a slug's current value (safer than set).
    ///
    /// Reads the diff from stdin or `--patch-file`. Refuses to apply if the
    /// slug has changed since `--base-hash` was captured, and refuses
    /// hunks whose context doesn't match the current value verbatim.
    Patch {
        slug: String,
        /// Read the patch from a file instead of stdin.
        #[arg(long)]
        patch_file: Option<String>,
        /// sha256 hex digest (lowercase) of the value the patch was generated
        /// against. Hashes the exact UTF-8 bytes returned by `buzz mem get`,
        /// not normalized lines. Run `buzz mem hash <slug>` to capture this
        /// before editing.
        #[arg(long)]
        base_hash: Option<String>,
        /// Skip the base-hash check. Unsafe if concurrent edits are possible —
        /// the patch will be applied against whatever the current value is,
        /// even if another agent rewrote it after the patch was generated.
        #[arg(long, default_value_t = false)]
        no_base_hash: bool,
        /// Echo the input patch + resulting sha256 and exit without writing.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Allow committing an empty result.
        #[arg(long, default_value_t = false)]
        allow_empty: bool,
        #[arg(long)]
        owner: Option<String>,
    },
    /// Publish a tombstone for a slug (cannot be used on `core`).
    Rm {
        slug: String,
        #[arg(long)]
        owner: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Pack subcommands (local, no relay connection needed)
// ---------------------------------------------------------------------------

/// Subcommands for `buzz pack`.
#[derive(Subcommand)]
pub enum PackCmd {
    /// Validate a persona pack directory
    Validate {
        /// Path to the pack directory
        path: String,
    },
    /// Inspect a persona pack — show metadata and effective config
    Inspect {
        /// Path to the pack directory
        path: String,
    },
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

async fn run(cli: Cli) -> Result<(), CliError> {
    let relay_url = client::normalize_relay_url(&cli.relay);

    // Pack commands are local-only — no relay connection needed.
    if let Cmd::Pack(ref sub) = cli.command {
        return match sub {
            PackCmd::Validate { path } => commands::pack::cmd_validate(path),
            PackCmd::Inspect { path } => commands::pack::cmd_inspect(path),
        };
    }

    // Auth: private key is required for all relay operations.
    // The keypair IS the identity — no tokens, no other auth.
    let private_key_str = cli.private_key.ok_or_else(|| {
        CliError::Auth("BUZZ_PRIVATE_KEY is required (use --private-key or set env var)".into())
    })?;
    let keys = Keys::parse(&private_key_str)
        .map_err(|e| CliError::Key(format!("invalid BUZZ_PRIVATE_KEY: {e}")))?;

    // NIP-OA: parse and verify the auth tag if provided.
    let (auth_tag, auth_tag_json) = match cli.auth_tag {
        Some(ref json) if !json.is_empty() => {
            let tag = buzz_sdk::nip_oa::parse_auth_tag(json)
                .map_err(|e| CliError::Auth(format!("BUZZ_AUTH_TAG is malformed: {e}")))?;
            buzz_sdk::nip_oa::verify_auth_tag(json, &keys.public_key()).map_err(|e| {
                CliError::Auth(format!(
                    "BUZZ_AUTH_TAG verification failed for pubkey {}: {e}",
                    keys.public_key().to_hex()
                ))
            })?;
            (Some(tag), Some(json.clone()))
        }
        _ => (None, None),
    };

    let client = BuzzClient::new(relay_url, keys, auth_tag, auth_tag_json)?;

    match cli.command {
        Cmd::Messages(sub) => commands::messages::dispatch(sub, &client, &cli.format).await,
        Cmd::Channels(sub) => commands::channels::dispatch(sub, &client, &cli.format).await,
        Cmd::Canvas(sub) => commands::channels::dispatch_canvas(sub, &client).await,
        Cmd::Reactions(sub) => commands::reactions::dispatch(sub, &client).await,
        Cmd::Emoji(sub) => commands::emoji::dispatch(sub, &client).await,
        Cmd::Dms(sub) => commands::dms::dispatch(sub, &client).await,
        Cmd::Users(sub) => commands::users::dispatch(sub, &client, &cli.format).await,
        Cmd::Workflows(sub) => commands::workflows::dispatch(sub, &client).await,
        Cmd::Feed(sub) => commands::feed::dispatch(sub, &client, &cli.format).await,
        Cmd::Social(sub) => commands::social::dispatch(sub, &client).await,
        Cmd::Notes(sub) => commands::notes::dispatch(sub, &client).await,
        Cmd::Repos(sub) => commands::repos::dispatch(sub, &client).await,
        Cmd::Patches(sub) => commands::patches::dispatch(sub, &client).await,
        Cmd::Issues(sub) => commands::issues::dispatch(sub, &client).await,
        Cmd::Upload(sub) => commands::upload::dispatch(sub, &client).await,
        Cmd::Mem(sub) => commands::mem::dispatch(sub, &client).await,
        Cmd::Pack(_) => unreachable!("handled above"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Smoke test: CLI definition is valid and parseable.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn command_inventory_is_stable() {
        let expected_groups: Vec<&str> = vec![
            "canvas",
            "channels",
            "dms",
            "emoji",
            "feed",
            "issues",
            "mem",
            "messages",
            "notes",
            "pack",
            "patches",
            "reactions",
            "repos",
            "social",
            "upload",
            "users",
            "workflows",
        ];

        let cmd = Cli::command();
        let mut actual: Vec<String> = cmd
            .get_subcommands()
            .map(|s| s.get_name().to_string())
            .filter(|n| n != "help")
            .collect();
        actual.sort();

        assert_eq!(
            actual.len(),
            expected_groups.len(),
            "Expected {} groups, got {}. Actual: {:?}",
            expected_groups.len(),
            actual.len(),
            actual
        );
        assert_eq!(
            actual, expected_groups,
            "Command group inventory drift detected"
        );
    }

    #[test]
    fn subcommand_names_are_stable() {
        fn names(cmd: &clap::Command, group: &str) -> Vec<String> {
            let group_cmd = cmd
                .get_subcommands()
                .find(|s| s.get_name() == group)
                .unwrap_or_else(|| panic!("group '{}' not found", group));
            let mut names: Vec<String> = group_cmd
                .get_subcommands()
                .map(|s| s.get_name().to_string())
                .filter(|n| n != "help")
                .collect();
            names.sort();
            names
        }

        let cmd = Cli::command();
        assert_eq!(
            names(&cmd, "messages"),
            vec![
                "delete",
                "edit",
                "get",
                "search",
                "send",
                "send-diff",
                "thread",
                "vote"
            ]
        );
        assert_eq!(
            names(&cmd, "channels"),
            vec![
                "add-member",
                "archive",
                "create",
                "delete",
                "get",
                "join",
                "leave",
                "list",
                "members",
                "purpose",
                "remove-member",
                "search",
                "set-add-policy",
                "topic",
                "unarchive",
                "update"
            ]
        );
        assert_eq!(names(&cmd, "canvas"), vec!["get", "set"]);
        assert_eq!(names(&cmd, "reactions"), vec!["add", "get", "remove"]);
        assert_eq!(
            names(&cmd, "emoji"),
            vec!["export", "import", "list", "rm", "set"]
        );
        assert_eq!(
            names(&cmd, "dms"),
            vec!["add-member", "hide", "list", "open"]
        );
        assert_eq!(
            names(&cmd, "users"),
            vec!["get", "presence", "set-presence", "set-profile"]
        );
        assert_eq!(
            names(&cmd, "workflows"),
            vec!["approve", "create", "delete", "get", "list", "runs", "trigger", "update"]
        );
        assert_eq!(names(&cmd, "feed"), vec!["get"]);
        assert_eq!(
            names(&cmd, "social"),
            vec![
                "contacts",
                "event",
                "list",
                "notes",
                "publish",
                "set-contacts",
                "set-list"
            ]
        );
        assert_eq!(names(&cmd, "repos"), vec!["create", "get", "list"]);
        assert_eq!(
            names(&cmd, "patches"),
            vec!["get", "list", "send", "status"]
        );
        assert_eq!(
            names(&cmd, "issues"),
            vec!["create", "get", "list", "status"]
        );
        assert_eq!(names(&cmd, "upload"), vec!["file"]);
        assert_eq!(names(&cmd, "pack"), vec!["inspect", "validate"]);
    }

    #[test]
    fn subcommand_counts_are_stable() {
        let expected: Vec<(&str, usize)> = vec![
            ("canvas", 2),
            ("channels", 16),
            ("dms", 4),
            ("emoji", 5),
            ("feed", 1),
            ("issues", 4),
            ("messages", 8),
            ("pack", 2),
            ("patches", 4),
            ("reactions", 3),
            ("repos", 3),
            ("social", 7),
            ("upload", 1),
            ("users", 4),
            ("workflows", 8),
        ];

        let cmd = Cli::command();
        for (group_name, expected_count) in &expected {
            let group = cmd
                .get_subcommands()
                .find(|s| s.get_name() == *group_name)
                .unwrap_or_else(|| panic!("group '{}' not found", group_name));
            let actual_count = group
                .get_subcommands()
                .filter(|s| s.get_name() != "help")
                .count();
            assert_eq!(
                actual_count, *expected_count,
                "Group '{}': expected {} subcommands, got {}",
                group_name, expected_count, actual_count
            );
        }
    }
}
