# Onboarding Spec — Step 7: Landing in the Main App

Source mockups (1280×832 each):

- `07a buzz welcome.png` — first landing: main app shell with a welcome/empty state
- `07b inbox.png` — the Inbox view populated with items
- `07c DM.png` — an open direct-message conversation

Step 7 is the **final onboarding step**: onboarding is complete and the user is
dropped into the real app shell. There is no modal/wizard chrome anymore — the
"onboarding" surface is now in-app guidance (welcome empty state, seeded
inbox items, and a starter DM) rather than a dedicated flow.

---

## 07a — Buzz Welcome (first landing)

### Overall layout

Standard two-region chat-app layout, identical in structure to the shipping
desktop app:

| Region | Width (approx.) | Notes |
|---|---|---|
| Left sidebar | ~250–260px (≈20% of 1280) | Full height, darker surface than main area |
| Main content | remaining ~1020px | Hosts the welcome empty state, vertically + horizontally centered |

The window is frameless in the mock (no visible native titlebar chrome beyond
the top strip); the sidebar runs edge-to-edge top to bottom.

### Sidebar (top → bottom)

1. **Workspace header** — workspace name ("Buzz") with a small mark/avatar at
   the top-left; acts as the workspace switcher affordance.
2. **Search / jump field** — full-width pill-shaped input directly under the
   header.
3. **Primary nav items** — icon + label rows:
   - **Inbox** (tray icon)
   - **Threads**
   - **Drafts / Saved** style utility row (dimmer weight)
4. **Channels section** — `Channels` section label (small caps, muted), then
   channel rows prefixed with `#`:
   - `# general`
   - `# random`
   - plus 1–2 additional channels; none selected/highlighted in 07a
   - a muted **"Add channel"** / `+` row at the bottom of the section
5. **Direct messages section** — `Direct Messages` label, then DM rows with
   small circular avatars + presence dots. In 07a these are the contacts the
   user connected during earlier onboarding steps (2–3 entries).
6. **User footer** — pinned to the bottom: the current user's avatar, display
   name, and a status/settings affordance (gear or chevron), separated from the
   list above by spacing rather than a hard divider.

Sidebar rows are ~32–36px tall with ~8–12px horizontal padding; section labels
have extra top margin (~16–20px) creating clear grouping.

### Main content — welcome empty state

Centered composition (both axes), max-width roughly 480–560px:

- **Illustration / logo** — the Buzz bee/logo mark rendered large above the
  heading (decorative, ~96px scale).
- **Heading** — `Welcome to Buzz` (large, bold, high-contrast text).
- **Subtext** — one to two lines of muted copy along the lines of
  *"You're all set. Pick a channel, check your inbox, or start a
  conversation."*
- **Getting-started actions** — a short stack/row of onboarding-completion
  callout cards or buttons beneath the subtext, each with an icon + label,
  covering the three next actions the flow wants to teach:
  1. **Check your Inbox** (maps to 07b)
  2. **Send a message / Start a DM** (maps to 07c)
  3. **Browse channels**
- No composer is shown in 07a — this is a home/empty view, not a channel view.

### Colors / theme

- Dark theme consistent with **Catppuccin Macchiato**:
  - Main background: deep blue-gray (≈ `#24273a` base)
  - Sidebar: slightly darker/mantle-toned surface (≈ `#1e2030`)
  - Primary text: near-white lavender (`#cad3f5` range)
  - Muted text: gray-blue subtext tones (`#a5adcb` / `#8087a2` range)
  - Accent: Buzz yellow/peach for the logo and highlight elements; blue/mauve
    for interactive accents (selection, links)
- Rounded corners throughout (~8px on cards/inputs), soft contrast, no hard
  borders — separation is done with surface-tone shifts.

### Spacing / sizing impressions

- Generous whitespace around the centered welcome block; the empty state
  occupies well under half the main area's width.
- Type ramp matches the app's rem scale: heading ≈ `text-2xl/3xl`, subtext ≈
  `text-base`, sidebar labels ≈ `text-sm`, section labels ≈ `text-xs`
  uppercase/tracking.

---

## 07b — Inbox (diff vs. 07a)

### What changes

- **Sidebar:** identical structure; the **Inbox** nav row is now the
  **active/selected item** — highlighted with a filled pill background
  (surface-tone) and brighter text/icon. An unread **count badge** appears on
  the Inbox row.
- **Main content:** the centered welcome state is replaced by a **two-pane
  inbox layout**:
  - **Inbox list pane** (left of main area, ~380–420px): header reading
    `Inbox` with a filter/"mark all read" affordance, then a vertical list of
    inbox items. Each item row contains:
    - circular avatar (or `#` channel glyph)
    - sender/channel name in medium weight
    - one-line message preview in muted text
    - right-aligned relative timestamp (e.g. "2m", "1h")
    - unread indicator (accent dot) on unread rows
    - Seeded items include a **welcome message from the Buzz team/bot** and
      mentions/DM notifications from the contacts added during onboarding.
  - **Detail/reading pane** (remaining width): shows either the selected inbox
    item's thread context or an inbox-specific empty prompt
    (*"Select a message to read it"* style muted copy centered in the pane).
- **No composer** in the inbox list itself; replying happens from the detail
  pane / by jumping to the conversation.

### What stays the same

Sidebar contents, theme, spacing system, and user footer are unchanged from
07a — this reads as pure in-shell navigation, not a new surface.

### Transition from 07a

Triggered by clicking the **Inbox** nav item (or the "Check your Inbox"
welcome action). The seeded welcome message gives the empty inbox immediate
content, teaching the inbox pattern on first use.

---

## 07c — DM (diff vs. 07a)

### What changes

- **Sidebar:** one **DM row is now selected** (highlighted pill) under Direct
  Messages; Inbox is no longer highlighted. The selected contact's presence
  dot is visible on their avatar.
- **Main content** becomes a full conversation view:
  - **Conversation header** (top bar, ~48–56px): contact avatar + display
    name, presence/status subline, and right-aligned actions (search-in-
    conversation, huddle/call, info).
  - **Message timeline**: left-aligned message groups — avatar, author name +
    timestamp header line, then message body text (`text-base`). The mock
    shows a short starter exchange (2–4 messages), including the first message
    the user sends after onboarding and/or a greeting from the contact. Day
    divider ("Today") separates the top of the timeline.
  - **Composer** pinned at the bottom: rounded input container with
    placeholder copy (`Message <name>` pattern), leading `+`/attach icon and
    trailing emoji + send icons. Send icon is accent-colored when the field
    has content.
- Timeline uses flat message rows (no bubbles) consistent with the desktop
  app's Slack-like layout; hover affordances (react/reply) implied but not
  the focus of the mock.

### What stays the same

Theme, sidebar structure, user footer, and overall proportions match 07a
exactly.

### Transition from 07a/07b

Reached by clicking a DM row in the sidebar, the "Start a DM" welcome action
in 07a, or opening a DM notification from the Inbox in 07b. This is the
terminal state of onboarding: the user has landed in a real conversation with
a working composer.

---

## How the three screens relate

```
07a Welcome  ──click "Inbox" nav / welcome action──►  07b Inbox
07a Welcome  ──click DM row / "Start a DM" action──►  07c DM
07b Inbox    ──open a DM notification───────────────►  07c DM
```

- **07a** is the handoff moment: onboarding wizard chrome is gone, and the
  main app shell (sidebar + content) is fully live. The welcome empty state
  doubles as the onboarding-completion callout.
- **07b/07c** demonstrate the two primary loops the onboarding wants the user
  to complete on day one — *triage the inbox* and *send a first message* —
  using content seeded during onboarding (welcome message, added contacts) so
  neither view is empty.
- All three share one shell; the only deltas are sidebar selection state and
  main-pane content, confirming these should be implemented as routes within
  the existing app shell (no special onboarding layout component needed), plus
  a first-run welcome empty state and inbox/DM seeding.

## Implementation notes for Buzz desktop

- Theme maps cleanly to the existing Catppuccin Macchiato dark palette — no
  new colors needed beyond the welcome illustration.
- The welcome state is a first-run route/empty-state component in the main
  content area; getting-started actions are plain buttons routing to Inbox /
  DM / channel browser.
- Seed data: a welcome DM/inbox item from a Buzz system identity should be
  created at onboarding completion so 07b/07c are non-empty.
- All text sizes should use rem tokens per the desktop text-sizing rules
  (`text-base` chat body, `text-2xs` meta text, etc.).
