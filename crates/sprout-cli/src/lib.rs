mod client;
mod commands;
mod error;
mod validate;

use clap::{Parser, Subcommand};
use client::SproutClient;
use error::CliError;
use nostr::Keys;

/// Run the Sprout CLI from raw arguments (including `argv[0]`).
///
/// Returns a process exit code (0 = success).
///
/// # Example
///
/// ```ignore
/// let code = sprout_cli::run_from_args(std::env::args()).await;
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
    name = "sprout",
    about = "Sprout CLI — interact with a Sprout relay",
    long_about = "\
Sprout CLI — interact with a Sprout relay

Configuration (flags override env vars):
  SPROUT_RELAY_URL     Relay base URL        [default: http://localhost:3000]
  SPROUT_PRIVATE_KEY   Nostr private key (hex or nsec)  [required]
  SPROUT_AUTH_TAG      NIP-OA auth tag JSON  [optional]

The 'pack' subcommand runs locally and does not require a relay connection.

Exit codes: 0=ok  1=bad input  2=relay/network error  3=auth error  4=other  5=write conflict
Errors are JSON on stderr: {\"error\": \"<category>\", \"message\": \"<detail>\"}"
)]
struct Cli {
    /// Relay URL (http:// or https://). Overrides SPROUT_RELAY_URL env var.
    #[arg(
        long,
        env = "SPROUT_RELAY_URL",
        default_value = "http://localhost:3000"
    )]
    relay: String,

    /// Nostr private key (hex or nsec). This is the CLI's identity.
    #[arg(long, env = "SPROUT_PRIVATE_KEY")]
    private_key: Option<String>,

    /// NIP-OA auth tag JSON (owner attestation). Injected into every signed event.
    #[arg(long, env = "SPROUT_AUTH_TAG")]
    auth_tag: Option<String>,

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

impl std::fmt::Display for PresenceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Online => write!(f, "online"),
            Self::Away => write!(f, "away"),
            Self::Offline => write!(f, "offline"),
        }
    }
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
    /// Announce and discover git repositories (NIP-34)
    #[command(subcommand)]
    Repos(ReposCmd),
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
        after_help = "Examples:\n  sprout messages send --channel <UUID> --content \"hello\"\n  sprout messages send --channel <UUID> --content \"@alice check this\""
    )]
    Send {
        /// Channel UUID (from 'sprout channels list')
        #[arg(long)]
        channel: String,
        /// Message text — supports @mentions and markdown
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
        /// Explicit mention pubkeys (64-char hex)
        #[arg(long = "mention")]
        mentions: Vec<String>,
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
        after_help = "Examples:\n  sprout messages get --channel <UUID>\n  sprout messages get --channel <UUID> --limit 50 --kinds 1,1984"
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
        /// Maximum reply depth to traverse
        #[arg(long)]
        depth_limit: Option<u32>,
        /// Maximum number of results to return
        #[arg(long)]
        limit: Option<u32>,
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
        after_help = "Examples:\n  sprout channels list\n  sprout channels list --visibility open"
    )]
    List {
        /// Filter by visibility
        #[arg(long, value_enum)]
        visibility: Option<ChannelVisibility>,
        /// Only show channels where the current identity is a member
        #[arg(long, default_value_t = false)]
        member: bool,
    },
    /// Get details for a single channel
    Get {
        /// Channel UUID
        #[arg(long)]
        channel: String,
    },
    /// Create a new channel
    #[command(
        after_help = "Examples:\n  sprout channels create --name general --type stream --visibility open\n  sprout channels create --name design --type forum --visibility open --description \"Design discussions\""
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
    },
    /// Update channel name or description
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
        /// Emoji character (e.g. '👍')
        #[arg(long)]
        emoji: String,
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
    #[command(after_help = "Examples:\n  sprout workflows trigger --workflow <UUID>")]
    Trigger {
        /// Workflow UUID
        #[arg(long)]
        workflow: String,
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
        after_help = "Examples:\n  sprout workflows approve --token <UUID>\n  sprout workflows approve --token <UUID> --approved false --note \"needs revision\""
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
        /// Comma-separated feed entry types to filter
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
    },
    /// Get a user's contact list
    #[command(name = "contacts")]
    GetContactList {
        /// 64-char hex pubkey.
        #[arg(long)]
        pubkey: String,
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

/// Subcommands for `sprout mem`.
#[derive(Subcommand)]
pub enum MemCmd {
    /// List non-tombstoned memory entries
    Ls {
        /// Owner pubkey (hex). Overrides SPROUT_AUTH_TAG.
        #[arg(long)]
        owner: Option<String>,
        /// Emit JSON instead of tab-delimited lines.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Print the value of a slug to stdout (no trailing newline)
    Get {
        slug: String,
        #[arg(long)]
        owner: Option<String>,
    },
    /// Set a slug's value. Pass `-` to read the value from stdin.
    Set {
        slug: String,
        value: String,
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

/// Subcommands for `sprout pack`.
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
        CliError::Auth("SPROUT_PRIVATE_KEY is required (use --private-key or set env var)".into())
    })?;
    let keys = Keys::parse(&private_key_str)
        .map_err(|e| CliError::Key(format!("invalid SPROUT_PRIVATE_KEY: {e}")))?;

    // NIP-OA: parse and verify the auth tag if provided.
    let (auth_tag, auth_tag_json) = match cli.auth_tag {
        Some(ref json) if !json.is_empty() => {
            let tag = sprout_sdk::nip_oa::parse_auth_tag(json)
                .map_err(|e| CliError::Auth(format!("SPROUT_AUTH_TAG is malformed: {e}")))?;
            sprout_sdk::nip_oa::verify_auth_tag(json, &keys.public_key()).map_err(|e| {
                CliError::Auth(format!(
                    "SPROUT_AUTH_TAG verification failed for pubkey {}: {e}",
                    keys.public_key().to_hex()
                ))
            })?;
            (Some(tag), Some(json.clone()))
        }
        _ => (None, None),
    };

    let client = SproutClient::new(relay_url, keys, auth_tag, auth_tag_json)?;

    match cli.command {
        Cmd::Messages(sub) => commands::messages::dispatch(sub, &client).await,
        Cmd::Channels(sub) => commands::channels::dispatch(sub, &client).await,
        Cmd::Canvas(sub) => commands::channels::dispatch_canvas(sub, &client).await,
        Cmd::Reactions(sub) => commands::reactions::dispatch(sub, &client).await,
        Cmd::Dms(sub) => commands::dms::dispatch(sub, &client).await,
        Cmd::Users(sub) => commands::users::dispatch(sub, &client).await,
        Cmd::Workflows(sub) => commands::workflows::dispatch(sub, &client).await,
        Cmd::Feed(sub) => commands::feed::dispatch(sub, &client).await,
        Cmd::Social(sub) => commands::social::dispatch(sub, &client).await,
        Cmd::Repos(sub) => commands::repos::dispatch(sub, &client).await,
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
            "feed",
            "mem",
            "messages",
            "pack",
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
                "topic",
                "unarchive",
                "update"
            ]
        );
        assert_eq!(names(&cmd, "canvas"), vec!["get", "set"]);
        assert_eq!(names(&cmd, "reactions"), vec!["add", "get", "remove"]);
        assert_eq!(names(&cmd, "dms"), vec!["add-member", "list", "open"]);
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
            vec!["contacts", "event", "notes", "publish", "set-contacts"]
        );
        assert_eq!(names(&cmd, "repos"), vec!["create", "get", "list"]);
        assert_eq!(names(&cmd, "upload"), vec!["file"]);
        assert_eq!(names(&cmd, "pack"), vec!["inspect", "validate"]);
    }

    #[test]
    fn subcommand_counts_are_stable() {
        let expected: Vec<(&str, usize)> = vec![
            ("canvas", 2),
            ("channels", 14),
            ("dms", 3),
            ("feed", 1),
            ("messages", 8),
            ("pack", 2),
            ("reactions", 3),
            ("repos", 3),
            ("social", 5),
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
