# Pulse prototype briefing

The For you briefing summarizes recent relay messages with the locally signed-in
Codex CLI. It describes intent rather than copying messages. For you shows only
up to ten substantive briefing highlights as individual summary rows with avatars, inline replies, and source
conversation links. Underline tabs across the top switch between Search, For you, DMs,
Channels, and Agents; Search contains the combined activity feed. Pulse hides the
global sidebar, while DMs and Channels retain their internal conversation lists.
Agent requests are inferred from posted messages, not an authoritative agent
runtime status.

This connection is for the local prototype. After the user authorizes sharing
their recent messages (including private conversations) with their Codex account,
set `BUZZ_PULSE_SUMMARY_PROVIDER=codex` in `desktop/.env.local`. The CLI must be on
the dev server's PATH, or specified by `BUZZ_PULSE_CODEX_BIN`. The switch is off by
default. No credential is embedded in the frontend.

Vite handles `/__pulse/briefing` only in development, for same-origin localhost
requests. This is not a relay endpoint and is not available in release builds.
The adapter uses an ephemeral, read-only Codex run with shell, apps, plugins,
web search, and multi-agent tools disabled. It has a 90-second deadline, bounded
input/output, and one concurrent run. Temporary response files are removed on
completion; the in-memory cache holds at most ten responses for five minutes.

The frontend submits at most 30 recent conversations, with up to eight messages
per conversation and 650 characters per message. A one-minute throttle limits
automatic updates. Summaries must reference supplied source IDs, and clicking a
highlight opens Search filtered to those conversations. Failures expose a retry action rather
than pretending an excerpt is an AI summary. Browser tests mock the model endpoint;
live checks require the user-approved Codex connection.

The ellipsis menu at the top right of the window switches between **Separate feeds**
and **Combined conversations**. The latter has no top tabs: Search and For you
appear as circular-icon rows above All messages in the persistent sidebar, and
Agents is hidden. All messages opens the mixed DM and channel feed above
a single 220px list of joined DMs and channels, sorted by the
newest relay timestamp or loaded message. Selection is independent of list order,
so incoming activity does not switch the open conversation. The variation and
selected conversation live in the URL and survive reloads; both variations use
the same conversation detail and draft storage within the 960px container.

Conversation detail uses message bubbles: incoming messages align left in gray,
and messages authored by the signed-in viewer align right in the selected theme’s accent color, with its matching contrast text. This is scoped
to Pulse through the message presentation context. Aggregate feed posts and
expanded replies share the bubble layout; generated briefing highlights retain
their summary presentation. Replies, reactions, attachments, editing,
and pending-send status use the existing message components and handlers.
Threads drill into the conversation area at every width; Back returns to the
conversation while the Pulse sidebar stays visible. Terminal sessions dock on the
right of the conversation. Opening a terminal, channel settings, profile, or agent
side panel expands the container to the available window width; closing the last
side panel restores the 960px limit. The terminal retains its sessions, keyboard
shortcut, maximize, and close controls. Narrow windows show it over the content.

The surrounding canvas uses the app’s glass-background setting. Message rows
have no hover fill. Reactions sit across the bubble’s upper-right edge; hover
actions share that corner (just above existing reactions so both remain usable),
with their position clamped to keep controls reachable on short messages.

Sidebar dots use the shell's unread channel and thread projections, backed by
the existing read markers. Recent activity, self-authored posts, and already-read
messages do not independently create dots. Reading a conversation advances its
channel marker; unread thread replies remain until their thread is read.

Pulse reuses the live channel/cache updates to refresh the aggregate feed in
one-second batches, with a follow-up pass if another update arrives during a
fetch. Returning to the app refreshes it too; reconnect recovery and the existing
30-second focused poll cover missed events and public notes. New conversations
appear automatically at the top; while scrolled down, a new-conversations button
preserves the reading position. The window's refresh button refreshes active
channel, thread, channel-list, and feed queries together.
