# Buzz Entity Links

Status: **partially implemented**. Done on this branch:

- Slice 0 — HTTPS relay git clone URLs (`{relay-origin}/git/<pubkey>/<repo>`)
  render as Buzz repository preview cards in chat
  (`desktop/src/shared/lib/linkPreview.ts`).
- Slice 1 — `buzz://pr|issue|repo` deep links: `entityLink.ts`
  builders/parser, preview cards with relay title enrichment, in-timeline
  click navigation to `/projects/$projectId`.
- Slice 3 (create-command part) — `crates/buzz-cli/src/links.rs`, `link`
  output field on `pr open` / `issues create` / `repos create`, base prompt
  guidance, cross-language golden-format tests.

Still unimplemented: OS-level deep links (slice 2), `link` on get commands,
the `buzz://project` scheme (waiting on NIP-MP landing), and the follow-ups
in slice 4.

## Problem

When a message contains a GitHub URL, the desktop client renders a rich
preview card ("GitHub · PR block/buzz #4020") below the message. Those cards
are produced entirely client-side by URL parsing in
`desktop/src/shared/lib/linkPreview.ts` and rendered by
`desktop/src/shared/ui/link-preview-attachment.tsx`.

Buzz-hosted entities have no equivalent. There is **no link format at all**
for a Buzz repository, project, pull request, or issue:

- The only rich deep link today is `buzz://message?channel=…&id=…`
  (`desktop/src/features/messages/lib/messageLink.ts`), rendered as an inline
  pill via `remarkMessageLinks.ts` + `MessageLinkPill.tsx`.
- OS-level deep links (`desktop/src-tauri/src/deep_link.rs`,
  `desktop/src/shared/deep-link.ts`) support `connect`, `join`,
  `add-community`, `message`, and `nostr-bind` — no git entities.
- `buzz pr open` / `buzz issues create` return raw event ids; there is no URL
  in their output and no guidance in the agent base prompt
  (`crates/buzz-acp/src/base_prompt.md`) for referencing Buzz work items in
  chat. Agents can only say "PR up" with a hex id.
- The relay-served web client only has `/repos/$repoId`; no PR/issue pages.

So an agent that opens a PR on a Buzz-hosted repository cannot produce
anything clickable, while the same agent opening a GitHub PR gets a card for
free.

## Goals

1. A canonical, shareable link format for Buzz repositories, projects, pull
   requests, and issues.
2. Rich preview cards in the desktop message timeline for those links, with
   parity to (and better data than) the GitHub cards — titles come from the
   actual Nostr events, not URL text.
3. Clicking a link navigates in-app to the existing project detail views.
4. CLI output includes the link so agents (and the base prompt) can emit it
   when announcing work.

## Non-goals (v1)

- Web (browser) pages for PRs/issues — the web client has no such views yet,
  so links are app-only, same as `buzz://message` today.
- Cross-community links. Like `buzz://message`, links are interpreted against
  the community the message was received in. A `relay=` query parameter is
  reserved for a future cross-community version but not emitted or consumed.
- Generic OpenGraph unfurling for arbitrary URLs — that is the separate
  `proto/rich-link-previews` prototype and stays orthogonal.
- Mobile rendering. Mobile should degrade gracefully (plain link) in v1;
  pill/card parity is a follow-up.

## Link format

Extend the existing `buzz://` scheme, mirroring `buzz://message`:

```
buzz://repo?owner=<pubkey-hex>&d=<repo-dtag>
buzz://project?owner=<pubkey-hex>&d=<project-dtag>
buzz://pr?id=<event-id-hex>&owner=<pubkey-hex>&d=<repo-dtag>
buzz://issue?id=<event-id-hex>&owner=<pubkey-hex>&d=<repo-dtag>
```

- `owner` is the 64-char lowercase hex pubkey of the repository/project
  announcement author (the NIP-34 / NIP-MP coordinate owner).
- `d` is the addressable `d`-tag. For `repo`/`project` links the
  (`owner`, `d`) pair is the full `30617:<owner>:<d>` /
  `30621:<owner>:<d>` coordinate.
- For `pr`/`issue` links, `id` identifies the kind `1618` / `1621` event;
  `owner` + `d` are the routing coordinate that lets the client navigate
  (and render a fallback card) without an event lookup. **v1 decision:** the
  implemented parser requires all three parameters — the CLI always emits
  them, and accepting hint-less links would force an event lookup before any
  navigation. A future revision can relax this without breaking existing
  links.

Validation rules match the existing codebase: `owner` and `id` are
`/^[a-f0-9]{64}$/`; `d` follows addressable d-tag rules already enforced in
`projectModels.ts` / `buzz-sdk`.

### HTTPS URLs

Agents naturally paste HTTPS clone URLs
(`{relay-origin}/git/<pubkey>/<repo>`) when announcing work, so those are
recognized **first** — implemented on this branch. Detection keys on the
path shape (`/git/` + 64-hex pubkey segment) rather than a host allow-list,
since relay hosts differ per community. The preview href is normalized to
the canonical `buzz://repo?owner=…&d=…` deep link (the raw transport URL is
not a browsable page), so clone-URL cards and inline clone-URL anchors get
the same in-app click navigation as explicit entity links, and both
spellings of the same repository dedupe to one card.

PRs, issues, and projects have no HTTPS page to link to (the web client has
no such routes), which is why they use the `buzz://` scheme above: it is
community-relative by construction, matches the established `buzz://message`
precedent, and requires no new relay surface. If web views land later, the
desktop can additionally recognize those `{relay-origin}/…` URLs with the
same card treatment.

## Rendering in chat (desktop)

Two presentations, consistent with how GitHub links and message links behave
today:

1. **Autolinked bare URL** (`<buzz://pr?…>` or bare in text): render an
   **attachment card** below the message in the existing `AttachmentGroup`,
   exactly like GitHub cards. Provider label `Buzz`, type label
   `PR` / `issue` / `repo` / `project`.
2. **Explicitly labeled markdown link** (`[fix the tooltip](buzz://pr?…)`):
   keep the author's label inline (same rule as
   `resolveMessageLinkRenderTarget` in `messageLink.ts`), still clickable.

### Card content and enrichment

Unlike GitHub (title derived from URL path only), Buzz entities live on the
same relay, so the card can show real data:

| Entity  | Title source                              | Fallback            |
|---------|-------------------------------------------|---------------------|
| PR      | `subject` tag of the kind `1618` event    | `PR <id-prefix>`    |
| Issue   | `subject` tag of the kind `1621` event    | `issue <id-prefix>` |
| Repo    | `name` tag of the kind `30617` event      | `d`-tag             |
| Project | `name` tag of the kind `30621` event      | `d`-tag             |

Enrichment is a single relay query by event id (PR/issue) or coordinate
(repo/project) through the existing `relayClient`, cached per event id.
Kind filters must always be included in the query (relay p-gate). Cards
render immediately with the fallback title and upgrade in place when the
lookup resolves — same progressive pattern as
`useResolvedLinkPreviews.ts` uses for Google titles.

Open/merged/closed status chips (from kind `1630`–`1633` status events) are
a nice-to-have and explicitly deferred to a follow-up.

### New module

`desktop/src/shared/lib/entityLink.ts` (placed in `shared/lib` rather than
the projects feature so `linkPreview.ts` — also `shared/lib` — can import
it without a feature→shared boundary violation):

- `buildRepoLink`, `buildPullRequestLink`, `buildIssueLink`
  (`buildProjectLink` deferred with the `project` scheme)
- `parseEntityLink(url): EntityLinkParseResult` (discriminated union, same
  shape as `parseMessageLink`)
- `isEntityLink(href)` cheap pre-check for the markdown renderer

Detection: extend `extractSupportedLinkPreviews` in `linkPreview.ts` with a
`buzz://` pattern (new `SupportedLinkPreviewKind` members
`buzz-pull-request`, `buzz-issue`, `buzz-repository`, `buzz-project`), or —
if mixing schemes into the URL regex is awkward — a parallel extractor
composed in `markdown.tsx`. Code blocks / spoiler / image-link masking rules
are shared either way, and the existing `MAX_PREVIEWS` cap applies across
both sources.

## Click handling and OS deep links

**In-timeline click** *(implemented)*: navigate via
`useAppNavigation.goProject()`. The `/projects/$projectId` route id is the
canonical `30617:<owner>:<d>` coordinate (see `entityLinkProjectRouteId` in
`shared/lib/entityLink.ts`). Route resolution on the `feat/multi-repository-projects`
branch (#4671) resolves this coordinate to the correct project and repository
regardless of container grouping — **#4671 must merge before #4695** to avoid
unresolved routes at runtime:

- `pr` / `issue` → `/projects/30617:<owner>:<d>?pullRequestId=<id>` (or `issueId`).
- `repo` → `/projects/30617:<owner>:<d>`.

If resolution fails (entity not visible in this community), show the same
kind of toast fallback used for unresolvable message links.

**OS-level**: register `repo` / `project` / `pr` / `issue` hosts in
`desktop/src-tauri/src/deep_link.rs` and dispatch to a new listener hook
(sibling to `useMessageDeepLinks.ts`). This makes links pasted outside Buzz
(e.g. in a terminal or another app) open the desktop app correctly.

## CLI (`buzz-cli`)

Add a `link` field to the JSON output of the write commands that create
linkable entities:

- `buzz pr open` → `{ event_id, accepted, message, link }`
- `buzz issues create` → same
- `buzz repos create` → link built from owner pubkey + `d`-tag
- `buzz projects create` → same

The builder lives in one Rust helper (e.g. `crates/buzz-cli/src/links.rs`)
so the format has exactly one definition on the Rust side; the TypeScript
`entityLink.ts` is its mirror and both are covered by shared-format tests
(golden strings asserted on both sides, like the NIP-MP fixture pattern).

`buzz pr get` / `buzz issues get` / `buzz repos get` also include `link` in
their output so agents can link to existing entities, not just ones they
just created.

## Agent guidance

One addition to `crates/buzz-acp/src/base_prompt.md`, next to the existing
`--channel` rule for PR opens:

> When you announce a pull request, issue, repository, or project in a
> channel message, include the `link` value from the command output as a
> bare URL on its own line so it renders as a preview card.

No persona changes needed — the base prompt applies to all managed agents.

## Interaction with existing work

- **`proto/rich-link-previews`** (generic OpenGraph cards): orthogonal.
  Entity links never hit the network beyond a relay event query; no overlap
  in code paths except the shared `AttachmentGroup` rendering slot.
- **`feat/multi-repository-projects` (NIP-MP)**: independent. Entity links
  reference single repositories/PRs/issues by coordinate/event id; the
  PR→project resolution step simply uses whatever project read models exist
  on `main` at implementation time.

## Implementation plan (suggested PR slices)

0. **HTTPS clone-URL repo cards** *(done, this branch)* — recognize relay
   `/git/<pubkey>/<repo>` URLs in `linkPreview.ts`, `Buzz` provider card
   with the `BuzzMark` logo, href normalized to the `buzz://repo` deep link
   for in-app navigation.
1. **Link core + cards** *(done, this branch)* — `entityLink.ts`, detection
   in `linkPreview.ts`, `Buzz` card variant in
   `link-preview-attachment.tsx`, in-timeline click navigation, relay title
   enrichment (with `resetLinkPreviewTitleCache()` wired into
   `resetCommunityState()`). Unit tests (`entityLink.test.mjs`, extended
   `linkPreview.test.mjs`).
2. **OS deep links** — `deep_link.rs` + listener hook + `deep-link.ts`
   parity tests.
3. **CLI + agent prompt** *(create commands done, this branch)* — `links.rs`
   helper, `link` output field on `pr open` / `issues create` /
   `repos create`, base prompt paragraph, cross-language golden-format test.
   Still open: `link` on the get commands.
4. **Follow-ups (separate)** — status chips on PR/issue cards, mobile
   pill/card rendering, web PR/issue routes + HTTPS link recognition,
   cross-community `relay=` parameter.

## Security considerations

- All identifiers are validated before use (`owner`/`id` strict hex-64,
  `d`-tag charset rules). Parse failures render the raw text as a plain,
  non-clickable string — never an anchor with an unvalidated href.
- Title enrichment queries go through the already-authenticated
  `relayClient` with explicit `kinds` filters; no new HTTP surface and no
  outbound fetches to third parties.
- Card titles come from event tags authored by arbitrary users; they must be
  rendered as text (existing card components already do this — verify no
  `dangerouslySetInnerHTML` in the new variant).
- Deep links arriving from the OS are untrusted input; the new listener must
  apply the same validation as the in-timeline parser before navigating.
