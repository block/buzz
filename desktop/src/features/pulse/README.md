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

The ellipsis menu at the right of the tabs switches between **Separate feeds**
and **Combined conversations**. The latter replaces the DMs and Channels tabs
with Conversations: All messages opens the mixed DM and channel feed at the top
of a single 200px list of joined DMs and channels, sorted by the
newest relay timestamp or loaded message. Selection is independent of list order,
so incoming activity does not switch the open conversation. The variation and
selected conversation live in the URL and survive reloads; both variations use
the same conversation detail and draft storage within the 800px container.
