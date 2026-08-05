# Buzz to Zorro Rebrand Plan

Status: Stage A implemented in the working tree; compatibility aliases included  
Prepared: 2026-08-04  
Repository snapshot: `54de2288`

## Implementation status — 2026-08-04

The repository-local portion of Stage A is implemented:

- user-visible product copy and package metadata now use **Zorro**;
- desktop, mobile, web, admin, installer, launch, favicon, and app-icon surfaces
  use a new Z mark, with canonical source artwork at
  `desktop/src-tauri/icons/zorro-source.svg`;
- the DMG background has matching Zorro artwork;
- desktop and mobile register and accept `zorro://` while continuing to accept
  `buzz://`; outbound builders intentionally continue emitting `buzz://` until
  the compatibility rollout permits the default to change;
- existing bundle IDs, keyring/data locations, lowercase crate/sidecar names,
  `buzz` CLI, `BUZZ_*` variables, persisted keys, deployment coordinates, and
  wire markers remain stable; the native desktop Cargo package and executable
  are now `zorro-desktop`, with legacy process-name recognition retained;
- existing documentation files remain unchanged.

Still external or approval-dependent: legal/trademark clearance, official
brand-asset approval, store/domain/repository changes, signed release pipelines,
deployment-repository changes, and any later Stage B/C namespace migration.

## Purpose

Rebrand the user-facing product from **Buzz** to **Zorro** without breaking
existing installations, identities, deep links, local data, relay deployments,
agent integrations, or release infrastructure.

This file is the only documentation artifact changed during planning. No
existing documentation is to be edited as part of the planning pass. Required
documentation work is recorded below for a later implementation phase.

## Recommended strategy

Use a staged rebrand, with a deliberate distinction between the product brand
and technical compatibility identifiers.

### Stage A: Zorro product, Buzz-compatible internals

Change what people see first:

- product name, titles, labels, onboarding copy, error messages, and alt text;
- application icons, logos, wordmarks, loading marks, favicons, launch images,
  installer artwork, and store/release presentation;
- website, desktop, mobile, and admin UI branding;
- new outbound links and marketing surfaces where compatibility permits.

Keep these stable initially:

- desktop and mobile bundle/application IDs;
- the existing OS keyring service;
- app-data, local-storage, shared-preference, and `~/.buzz` paths;
- `buzz://` link acceptance;
- existing `buzz` CLI and non-desktop `buzz-*` executables/crates;
- `BUZZ_*` environment variables;
- Nostr tag names, protocol markers, Redis topics, metrics, and database fields;
- existing container, Helm, repository, and package coordinates.

This produces a low-risk branded release that upgrades in place.

### Stage B: compatibility aliases

Introduce new Zorro-facing interfaces while retaining Buzz aliases:

- register and emit `zorro://`, but continue accepting `buzz://`;
- optionally ship a `zorro` CLI alias while retaining `buzz`;
- optionally accept `ZORRO_*` configuration with documented precedence over
  `BUZZ_*`, retaining the old variables for at least one full deprecation
  window;
- migrate local keys and paths with copy-on-read or one-time transactional
  migration, never by simply switching constants;
- publish new artifact aliases before changing any default download or deploy
  path.

### Stage C: optional technical namespace rename

Rename internal crates, executables, repository paths, deployment resources,
wire markers, and persisted namespaces only if there is a concrete maintenance
or product benefit. This is a separate engineering program, not required to
complete the visible rebrand.

## Decisions required before implementation

Resolve these before replacing assets or product identifiers:

1. Confirm legal/trademark clearance for **Zorro** in all distribution regions.
2. Confirm ownership and availability of intended domains, social identities,
   App Store name, Play Store name, GitHub organization/repository path, and
   container/package coordinates.
3. Approve the exact written brand:
   - display name: `Zorro`;
   - lowercase command/technical prefix: `zorro`;
   - capitalization in prose and possessives;
   - whether “Buzz” remains in compatibility notices.
4. Approve a visual system and asset package:
   - primary mark and wordmark;
   - monochrome/current-color mark for light and dark themes;
   - square application icon with safe-zone definition;
   - adaptive Android foreground/background layers;
   - launch/loading animation behavior;
   - installer/DMG artwork;
   - favicon and small-size mark;
   - light, dark, high-contrast, and reduced-motion behavior.
5. Decide whether bee-specific product language and characters remain. The
   current experience contains bee artwork, flapping animations, honey/bumble
   naming, and a “Buzz” theme. Zorro needs an explicit decision for each rather
   than an accidental mixture of brands.
6. Decide whether the initial rebrand must upgrade the existing Buzz app in
   place. The recommendation is **yes**, which means retaining current bundle
   IDs and signing lineage for the first Zorro release.
7. Define the compatibility support period for `buzz://`, `buzz`, `BUZZ_*`,
   current domains, and deployment artifacts.

## Brand surface inventory

### 1. Desktop logo system

The desktop mark is code-driven as well as file-based. Replacing only PNGs will
leave the bee throughout the application.

| Surface | Current location | Future action |
|---|---|---|
| Static vector mark | `desktop/src/shared/ui/buzz-logo/BuzzMark.tsx` | Replace geometry with an approved current-color Zorro mark; expose a brand-neutral component name. |
| Animated morph/texture | `desktop/src/shared/ui/buzz-logo/BuzzLogoAnimation.tsx` | Redesign animation around the Zorro mark or replace it with a simpler transition; update accessible labels. |
| Animation styling | `desktop/src/shared/ui/buzz-logo/buzz-logo-animation.css` | Preserve reduced-motion handling; aliases can avoid a high-risk class-name sweep in the first release. |
| Flapping bee | `desktop/src/shared/ui/buzz-logo/FlappingBee.tsx` | Replace with an approved Zorro loading mark/animation; the current implementation has WebKit compositor-specific behavior that the replacement must preserve or intentionally remove. |
| Fuzzy mark wrapper | `desktop/src/shared/ui/buzz-logo/FuzzyLogo.tsx` | Rename to a neutral branded loader/mark abstraction and update default ARIA labels. |
| Landing bee composition | `desktop/src/features/onboarding/ui/LandingBees.tsx` | Redesign or remove the multi-bee onboarding scene. |
| Boot/loading gates | `desktop/src/app/App.tsx` | Replace `BuzzMark`, `FuzzyLogo`, and `FlappingBee` usage while preserving first-paint and E2E timing behavior. |
| Onboarding chrome | `desktop/src/features/onboarding/ui/OnboardingChrome.tsx` | Replace embedded mark. |
| Setup and backup | `desktop/src/features/onboarding/ui/SetupStep.tsx`, `BackupStep.tsx` | Replace animated marks and brand copy. |
| Invite/loading gate | `desktop/src/features/onboarding/ui/PendingInviteGate.tsx` | Replace animated bee. |
| Hosted community onboarding | `desktop/src/features/communities/ui/HostedCommunityOnboarding.tsx` | Replace mark and visible brand copy. |
| Huddle loading | `desktop/src/features/huddle/components/HuddleStartingView.tsx` | Replace flapping bee without changing huddle startup semantics. |
| Agent activity indicators | `desktop/src/features/agents/ui/TurnLivenessIndicator.tsx`, `AgentSessionTranscriptList.tsx` | Decide whether the product mark should remain an activity indicator; replace if so. |
| Repo/link fallback marks | `desktop/src/features/projects/ui/RepositoryCards.tsx`, `desktop/src/shared/ui/link-preview-attachment.tsx` | Replace mark used for Buzz-hosted entities. |
| Runtime fallback mark | `desktop/src/features/onboarding/ui/RuntimeIcon.tsx` | Replace the app mark used when a runtime has no own logo. |

Create a single brand component API so future artwork changes do not require
feature-level imports from a `buzz-logo` directory. Suggested conceptual API:

```text
BrandMark
BrandWordmark
BrandLoader
BrandAppIcon
```

The implementation may retain old CSS class names temporarily for compatibility
and smaller diffs; class names are not themselves user-visible.

### 2. Desktop bitmap/vector assets and packaging artwork

| Asset group | Current files | Notes |
|---|---|---|
| Public app icons | `desktop/public/app-icon@2x.png`, `desktop/public/app-icon@3x.png` | Used in pairing/identity UI and copied into the browser bundle. Replace at the same dimensions or update consumers/tests together. |
| Public SVG | `desktop/public/buzz.svg` | Used by rendered content and E2E fixture data. Introduce a Zorro asset, then retain the old path temporarily if stored content may reference it. |
| Landing wordmark | `desktop/public/landing/buzz-wordmark.png` | Replace with approved Zorro wordmark; update filename and import in the same change. |
| Icon source | `desktop/src-tauri/icons/buzz-source.png` | Treat as the source of truth for regenerated platform icons. Do not hand-replace every generated size without recording the generation command. |
| Desktop icons | `desktop/src-tauri/icons/32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.png`, `icon.icns`, `icon.ico` | Regenerate from the approved source and visually verify every platform. |
| Windows tiles | `desktop/src-tauri/icons/Square*Logo.png`, `StoreLogo.png` | Regenerate and verify transparent padding and small-size legibility. |
| Tauri Android icons | `desktop/src-tauri/icons/android/**` | Replace adaptive foreground, round, legacy, and background resources if this target remains supported. |
| Tauri iOS icons | `desktop/src-tauri/icons/ios/**` | Regenerate the complete icon set if this target remains supported. |
| DMG background | `desktop/src-tauri/icons/dmg-background.png` | Inspect and replace branded artwork while preserving current 1320x1000 layout and icon/drop-target positions. |
| Social/card template | `desktop/src-tauri/assets/card_template.png` | Visually inspect for embedded Buzz artwork or text before release. |

Third-party harness/runtime logos under `desktop/public/harness-logos/`,
`desktop/public/runtime-icons/`, and
`desktop/src/features/onboarding/assets/harness-logos/` identify external tools
and should not be rebranded as Zorro.

### 3. Web and admin artwork

| Surface | Current location | Future action |
|---|---|---|
| Browser app icon | `web/src/assets/app-icon@3x.png` | Replace from the same approved app-icon source used by desktop. |
| Invite page | `web/src/features/invite/ui/InvitePage.tsx` | Replace icon, `alt="Buzz"`, copy, and new-app deep-link emission. |
| Repository page | `web/src/features/repos/ui/ReposPage.tsx` | Replace icon and alt text. |
| Browser title | `web/index.html` | Change visible title in the documentation/copy phase. |
| Admin inline mark | `admin-web/src/App.tsx::BuzzMark` | Replace the duplicated inline bee SVG with the approved mark; preferably share/generated source to avoid drift. |
| Admin favicon | `admin-web/public/favicon.svg` | Replace the honeycomb icon. |
| Admin title/copy | `admin-web/index.html`, `admin-web/src/App.tsx` | Replace visible title and descriptions. |

### 4. Mobile artwork

| Surface | Current location | Future action |
|---|---|---|
| In-app Buzz icon | `mobile/assets/images/buzz-icon.png` | Replace and rename; update `pubspec.yaml` and all consumers. |
| Painted bee animation | `mobile/lib/shared/widgets/tappable_flapping_bee.dart` | Replace with a Zorro animation or static mark while retaining tap semantics and reduced-motion/accessibility behavior. |
| Pairing welcome | `mobile/lib/features/pairing/pairing_page/pairing_welcome_view.dart` | Replace the painted bee and visible copy. |
| Android icons | `mobile/android/app/src/main/res/mipmap-*/ic_launcher*.png` | Regenerate legacy, round, foreground, and adaptive icon resources. |
| Android icon configuration | `mobile/android/app/src/main/res/mipmap-anydpi-v26/ic_launcher.xml`, `values/ic_launcher_background.xml`, `drawable/ic_launcher_foreground_inset.xml` | Update background/foreground references and approved colors. |
| Android launch images | `mobile/android/app/src/main/res/mipmap-*/launch_image.png`, `drawable*/launch_background.xml` | Replace and verify light/dark launch behavior. |
| iOS AppIcon set | `mobile/ios/Runner/Assets.xcassets/AppIcon.appiconset/**` | Regenerate every declared size from the approved source. |
| iOS launch images | `mobile/ios/Runner/Assets.xcassets/LaunchImage.imageset/**` | Replace all scales and update the asset README later. |

### 5. Legacy or ambiguous artwork

Inspect and classify these before deleting or replacing them:

- `crates/buzz-agent/sprout-agent.png`;
- `docs/assets/sprout.png` and `docs/assets/sprout-icon.png`;
- current documentation screenshots under `docs/assets/screenshots/`;
- starter-team portraits and bee-themed names under
  `desktop/public/onboarding/starter-team/`;
- theme screenshot fixtures such as
  `desktop/tests/e2e/buzz-theme-screenshots.spec.ts`.

Historical screenshots and changelogs should generally remain historical.
Current product screenshots, examples, and onboarding artwork should be
recaptured after the Zorro UI is final.

## User-facing name and copy inventory

Perform a semantic copy pass rather than a blind replacement. Current visible
Buzz strings occur in:

- desktop onboarding, welcome messages, settings, profile/identity binding,
  terminal labels, agent setup, Git errors, notifications, community setup,
  shared compute, local archive, and reset/sign-out flows;
- Tauri menus, notifications, native errors, window/process diagnostics, and
  permission messages;
- mobile pairing, channels, sharing, downloads, settings, and app title;
- web invite/repository pages and admin descriptions;
- relay NIP-11 descriptions, CLI help, agent prompts, MCP tool descriptions,
  generated system messages, and release metadata.

Classify each match before editing:

| Class | Example | Treatment |
|---|---|---|
| Visible product copy | “Welcome to Buzz” | Change to Zorro. |
| Accessibility | `alt="Buzz"`, “Buzz logo” | Change to Zorro and verify semantics. |
| User-visible feature name | “Buzz Term”, “Buzz theme” | Product/design decision; rename consistently if these are part of the master brand. |
| Compatibility explanation | “Open this Buzz link” | Update wording while retaining support for the old interface. |
| Internal symbol/class | `BuzzMark`, `.buzz-logo` | Rename opportunistically; not a launch blocker if hidden. |
| Historical record | changelog, old release name | Preserve. |
| Third-party or example data | repo names, fake email addresses | Change only if it presents as current first-party branding. |

Use centralized brand constants for simple runtime text where it improves
consistency, but do not build phrases through string concatenation merely to
avoid literals. Localization-ready complete strings are preferable.

## Product and package metadata

### Desktop

Implemented primary metadata:

- `desktop/src-tauri/tauri.conf.json`
  - `productName: "Zorro"`;
  - identifier `xyz.block.buzz.app`;
  - deep-link schemes `buzz` and `zorro`;
  - sidecars named `buzz-*` and `buzz`.
- `desktop/package.json`: package name `buzz`.
- `desktop/src-tauri/Cargo.toml`: package and executable `zorro-desktop`, with
  the internal library still named `buzz_lib`.
- `scripts/instance-env.sh`: `Zorro Dev` product names and
  `xyz.block.buzz.app.dev*` identifiers.

First Zorro release recommendation:

- change `productName` and visible dev names to Zorro;
- retain `xyz.block.buzz.app` so the signed release upgrades the existing app
  and retains its app-data container;
- add `zorro` as a registered scheme while retaining `buzz`;
- retain sidecar executable names until aliases and discovery logic exist;
- keep the existing keyring service and migrate only in a dedicated,
  heavily-tested change.

Changing the bundle ID immediately would normally install a separate app and
can sever access to existing app data, permissions, updater lineage, and
keychain entries. If a new identifier is mandatory, design and test a signed
migration helper and keychain access-group strategy before release.

### Mobile

Current primary metadata:

- `mobile/pubspec.yaml`: package `buzz`, description, and Buzz icon asset;
- Android namespace/application ID `xyz.block.buzz.mobile` and display name
  `Buzz` in `mobile/android/app/build.gradle.kts`;
- Android Kotlin package paths under `xyz/block/buzz/mobile`;
- iOS `CFBundleName` and generated `APP_DISPLAY_NAME` values;
- iOS product bundle identifier in the Xcode project/build settings;
- Android and iOS registration for the `buzz` URL scheme.

First Zorro release recommendation:

- change only display name and artwork;
- keep Android application ID and iOS bundle ID for in-place store updates;
- register/accept both URL schemes;
- do not move Kotlin package paths merely for visible branding;
- coordinate push entitlements, App Attest app ID, APNs topic, and signing
  profiles before any later bundle-ID change.

### Browser/admin

Update titles, manifest-like metadata if introduced, favicons, icons, alt text,
and download names. Preserve old download parsing during the transition so the
web client can recognize both Buzz and Zorro artifacts.

## Compatibility-sensitive interfaces

### Deep links

`buzz://` is a public protocol used by desktop, mobile, web handoff pages, the
CLI, ACP prompts, Markdown rendering, Git entity previews, pairing, invites,
tests, and external messages.

Current link families include:

- `buzz://message`;
- `buzz://join`;
- `buzz://connect`;
- `buzz://add-community`;
- `buzz://nostr-bind`;
- `buzz://repo`, `buzz://pr`, and `buzz://issue`;
- legacy pairing payloads beginning with `buzz://`.

Migration plan:

1. Register both `buzz` and `zorro` schemes on desktop, Android, and iOS.
2. Refactor parsers to accept an explicit supported-scheme set.
3. Keep parsing and security validation identical for both schemes.
4. Switch newly generated links to `zorro://` only after released clients on
   every supported platform accept it.
5. Keep `buzz://` acceptance indefinitely unless telemetry proves it can be
   retired; messages and external sites can retain old links forever.
6. Add cross-platform fixtures proving every link family works under both
   schemes and malformed links remain rejected.

Do not globally replace protocol strings. Identity-binding values such as
`buzz:nostr-identity`, callback expectations, and signed audiences are protocol
values that require coordinated versioning.

### Local data and secrets

Desktop persists data under many `buzz-*` local-storage keys, including
communities, active community, onboarding, themes, drafts, unread state,
channel snapshots, navigation, notification settings, preferences, and cached
icons. Mobile similarly uses `buzz_*`, `buzz.*`, and `buzz-*` preference keys.

Desktop additionally uses:

- OS keyring service `buzz-desktop` and dev variants;
- production nest `~/.buzz` and development nest `~/.buzz-dev`;
- Tauri app-data directories derived from `xyz.block.buzz.app*`;
- process markers and orphan-reaper matching tied to current executable and
  identifier names.

Migration rules:

- keep existing keys and paths during Stage A;
- if renamed later, read new key first, then old key, validate, write new key,
  and remove the old key only after a durable verification step;
- do not migrate secret/keyring state in the same release as bundle IDs unless
  an end-to-end rollback plan exists;
- preserve reset/sign-out coverage across both old and new locations;
- test upgrade, downgrade, interrupted migration, locked/unavailable keyring,
  multiple worktrees, and side-by-side dev builds;
- never silently generate a new Nostr identity because a renamed keyring slot
  appears empty.

### CLI, agents, and executables

The repository exposes a `buzz` CLI plus numerous `buzz-*` binaries. Agent
prompts and managed environments assume those names.

Recommended transition:

1. Keep existing CLI and sidecar names; the native desktop executable is the
   deliberate exception and is now `zorro-desktop`.
2. Add `zorro` as an alias/symlink/wrapper for `buzz` after confirming packaging
   behavior on macOS, Linux, and Windows.
3. Make user-facing help say Zorro while documenting the compatibility command.
4. If sidecars receive new aliases, update Tauri bundling, binary discovery,
   managed-agent snapshots, process cleanup, install reports, and CI stubs as
   one coordinated change.
5. Treat Rust crate/module renames as optional cleanup; they create broad lock
   file, workspace, import, cache, CI, and downstream dependency churn without
   improving the visible product.

Desktop upgrade compatibility recognizes `Buzz`, `buzz-desktop`,
`buzz_desktop`, and the truncated AppImage process name `buzz-desktop.bi` so
managed-agent orphan cleanup remains safe across an in-place upgrade.

### Environment variables and configuration

There are many `BUZZ_*` variables across relay, desktop, mobile release,
agents, media, Git, mesh, push, CI, and deployment scripts.

Recommended transition:

- keep `BUZZ_*` fully supported in Stage A;
- if `ZORRO_*` is introduced, define deterministic precedence:
  `ZORRO_*` wins, `BUZZ_*` is the fallback, conflicting values produce a
  bounded warning without printing secrets;
- add one shared helper per process rather than duplicating alias logic;
- retain secret names and Kubernetes values until rollout tooling supports both;
- inventory environment variables automatically and test that every intended
  alias resolves identically.

### Protocol, persistence, and observability namespaces

The string `buzz` also appears in Nostr tags, relay-signed markers, invite key
derivation labels, Redis topics, database columns/metadata, metrics, tracing
targets, tool names, Git metadata, email-like identities, and push audiences.

Default treatment: **preserve these as protocol/internal identifiers**.

Renaming any of them may split data or invalidate signatures/capabilities. A
technical rename requires its own versioned protocol proposal and dual-read or
dual-publish rollout. In particular, do not casually change:

- signed audience/action/protocol values;
- invite-token derivation labels;
- push-gateway audiences and APNs/App Attest identity;
- Nostr tags such as relay workflow markers and Git metadata;
- Redis channel/topic prefixes;
- Prometheus metric names used by dashboards and alerts;
- stored agent tool names or transcript parsing aliases.

## Domains and hosted services

Current first-party domain coupling includes:

- hosted communities under `*.communities.buzz.xyz`;
- push delivery at `push.buzz.xyz`;
- pairing relay examples/defaults under `pairing.buzz.xyz`;
- contributor/security email at `buzz-relay.org`;
- relay URLs, NIP-05 examples, updater/download links, and website callbacks.

Domain rollout order:

1. Acquire and configure Zorro domains and certificates.
2. Make backend tenant/host registries explicitly recognize both old and new
   domains without changing a community ID.
3. Add new DNS/ingress routes and verify auth, NIP-98 URL binding, CORS, media,
   Git, WebSockets, invites, callbacks, and push audiences.
4. Update clients to prefer Zorro domains.
5. Retain redirects or service aliases for old HTTPS hosts and direct service
   support for old WebSocket hosts; WebSocket clients do not benefit from
   ordinary browser redirects in every implementation.
6. Migrate hosted community URLs with an explicit ownership and canonical-host
   policy. Do not let a hostname change create a new tenant or cross-tenant
   lookup.
7. Retire old domains only after telemetry, external integration review, and a
   documented recovery path.

## Repository, release, and deployment surfaces

Current Buzz coordinates include:

- GitHub repository `block/buzz` and related internal repositories;
- images such as `ghcr.io/block/buzz`, `buzz-push-gateway`, and `buzz-sprig`;
- Helm charts under `deploy/charts/buzz*` and OCI chart paths;
- Docker Compose project, container, volume, and network names;
- Rust crates and binaries named `buzz-*`;
- desktop/mobile release artifact names containing `Buzz_` or `buzz-`;
- workflow artifacts, cache keys, attestations, CODEOWNERS, and release scripts;
- default namespaces, Secrets, Services, labels, and alert names.

Roll these out as aliases before renaming defaults:

1. Publish Zorro-branded desktop/mobile filenames while keeping release parsing
   able to recognize the former names.
2. If a new container coordinate is required, publish the same digest to both
   coordinates during the transition and verify signatures/attestations at
   both names.
3. Keep existing Helm release compatibility. Renaming Kubernetes resources can
   cause replacement rather than an in-place upgrade, especially StatefulSets,
   PVCs, Secrets, Services, and selectors.
4. Treat chart-name or directory changes as a chart migration with rendered
   manifest diffs and rollback tests.
5. Update external repositories and deployment consumers only after aliases are
   live.
6. Preserve historical release tags and changelogs.

## Implementation phases

### Phase 0: brand contract and compatibility ADR

Deliverables:

- legal/name approval;
- domain and store-name reservation;
- approved asset kit and usage guide;
- compatibility matrix for identifiers and deprecation periods;
- explicit decision on bee language, starter characters, and theme names;
- baseline screenshots and a machine-generated brand-string inventory;
- architecture decision record for bundle IDs, deep links, storage, CLI, env,
  domains, protocol namespaces, and deployment coordinates.

Exit criterion: every category is marked **rename now**, **alias**, **migrate
later**, **preserve**, or **historical**.

### Phase 1: central brand primitives

Deliverables:

- brand-neutral desktop components (`BrandMark`, `BrandLoader`, etc.);
- one approved icon source and reproducible generation scripts/commands;
- shared asset outputs for desktop, web, admin, Android, and iOS;
- centralized product display-name constants where useful;
- accessibility and reduced-motion tests;
- no change yet to technical identifiers.

Exit criterion: all current bee/logo render paths can consume the new Zorro
assets through a small set of primitives.

### Phase 2: desktop visible rebrand

Deliverables:

- replace logos, onboarding scenes, wordmark, loader, app icons, DMG artwork,
  native menu/notification copy, and visible strings;
- change Tauri product display name but retain bundle ID/keyring/app-data
  continuity;
- update snapshot, helper, and E2E expectations;
- capture scoped screenshots of cold boot, onboarding, main app, huddle,
  settings/pairing, native installer, tray/menu, and notifications.

Exit criterion: no unintended user-visible Buzz branding remains in desktop,
while an installed Buzz release upgrades to Zorro with the same identity and
communities.

### Phase 3: mobile visible rebrand

Deliverables:

- replace Flutter mark, app/launch icons, titles, pairing artwork, copy, and
  accessibility labels;
- retain Android application ID and iOS bundle ID;
- verify App Store/Play Store update and signing paths;
- test notification badge, push, camera/photo permissions, sharing, deep links,
  launch screens, light/dark themes, and upgrade persistence.

Exit criterion: store-distributed upgrades preserve mobile identity and local
state while presenting Zorro throughout the UI.

### Phase 4: web and admin visible rebrand

Deliverables:

- replace icons, inline SVG, favicon, titles, alt text, invite copy, repository
  copy, admin descriptions, and download presentation;
- ensure relay static bundle routing is unchanged;
- verify invitation-to-app and connect-to-app handoffs.

Exit criterion: public browser surfaces present Zorro and remain compatible
with currently released Buzz clients.

### Phase 5: compatibility aliases

Deliverables:

- dual `zorro://`/`buzz://` registration and parsing;
- Zorro link generation only after dual-scheme clients are deployed;
- optional `zorro` CLI alias;
- optional `ZORRO_*` configuration aliases with tested precedence;
- old domain support plus Zorro domain preference;
- migration telemetry that contains no secrets or high-cardinality labels.

Exit criterion: new Zorro interfaces work end to end and every supported old
Buzz interface still works.

### Phase 6: release and infrastructure branding

Deliverables:

- Zorro artifact filenames, update metadata, release titles, image/chart aliases,
  store listings, signing/notarization adjustments, and external-repo updates;
- in-place Helm and Compose upgrade tests;
- signed desktop and mobile upgrade evidence;
- operational dashboard/alert annotations for intentionally retained `buzz_*`
  metric names.

Exit criterion: production artifacts are marketed as Zorro without breaking
existing automation or replacing stateful infrastructure.

### Phase 7: documentation and launch communications

No documentation changes are part of the current planning task. Later, update
current documentation in a dedicated pass, including:

- `README.md`, `AGENTS.md`, `CONTRIBUTING.md`, `ARCHITECTURE.md`, `TESTING.md`,
  `RELEASING.md`, vision documents, and `docs/CODEBASE_CONTEXT.md`;
- crate/client READMEs, CLI testing guides, examples, deployment guides, Helm
  README/examples, NIP extension prose, agent prompts, persona specifications,
  screenshot captions, issue/PR templates, and release workflows;
- external documentation in `buzz-releases`, `sprout-oss`,
  `block-coder-tf-stacks`, and `sprout-backend-blox`;
- website, store listings, security/contact pages, contributor links, and
  engineering posts.

Preserve historical changelogs, old release notes, old issue/PR titles, and
versioned protocol specifications unless a correction is required. Add a short
“Zorro was formerly Buzz” compatibility note during the transition.

### Phase 8: optional internal rename

Only after the public rebrand is stable, evaluate renaming:

- repository and workspace package names;
- Rust crates, libraries, and sidecar binaries;
- Docker/Helm/Kubernetes coordinates;
- environment variables and local namespaces;
- metrics and tracing targets;
- Nostr/protocol markers.

Require a benefit/risk analysis per category. “The internal name is old” is not
by itself enough to justify breaking downstream consumers or rewriting durable
protocol identifiers.

## Suggested pull-request sequence

Keep changes reviewable and reversible:

1. Brand primitives and asset-generation pipeline.
2. Desktop mark/loader/onboarding visuals.
3. Desktop copy and native packaging metadata.
4. Desktop platform icons and installer artwork.
5. Mobile Flutter artwork/copy.
6. Mobile Android/iOS icons and display metadata.
7. Web/admin artwork and copy.
8. Dual deep-link support, with old scheme still emitted.
9. Switch new link emission to `zorro://` after compatible releases exist.
10. Domain aliases and hosted-service migration.
11. Release/container/chart aliases.
12. Documentation and screenshots.
13. Optional CLI/env/internal namespace work.

Do not combine bundle-ID, keyring, app-data, deep-link, domain, and executable
renames in one pull request.

## Verification matrix

### Static inventory gates

Use case-insensitive searches, then classify every remaining match:

```bash
rg -n -i '\bBuzz\b|buzz-logo|FlappingBee|LandingBees|buzz-wordmark'
rg -n 'buzz://|xyz\.block\.buzz|buzz-desktop|\.buzz|BUZZ_[A-Z0-9_]+'
rg --files | rg -i 'buzz|sprout|logo|icon|favicon|wordmark|launch|splash|dmg'
```

Expected remaining matches must be allowlisted by category: compatibility,
internal-only, protocol, historical, or third-party/example.

### Upgrade and data safety

Test at minimum:

- install the last Buzz release, create/import an identity, add multiple
  communities, create drafts/preferences/read state, then upgrade to Zorro;
- verify the pubkey, secrets, communities, nest, agent definitions, archives,
  drafts, read state, media cache behavior, and settings are preserved;
- verify reset and sign-out remove all intended old and new locations;
- simulate unavailable/corrupt keyring during upgrade and confirm no silent
  identity rotation;
- verify worktree/dev variants remain isolated;
- verify downgrade behavior while old releases remain supported.

### Link compatibility

For every supported link family, test:

- old scheme into new desktop;
- new scheme into new desktop;
- old scheme into new mobile where supported;
- new scheme into new mobile where supported;
- browser invite/connect handoff;
- CLI-generated repo/PR/issue/message links;
- percent encoding, hostile authorities, malformed URLs, unsupported hosts,
  callbacks, and signed identity-binding fields.

### Visual QA

Verify:

- 16/20/29/32/40/44/60/76/83.5/128/256/512/1024-class icon sizes as
  applicable;
- macOS dock, menu, notifications, DMG, installed app, dark mode, and Retina;
- Windows executable/installer, taskbar, Start tiles, and high-DPI behavior;
- Linux AppImage/deb icons and desktop entries;
- Android adaptive/round/legacy icons and splash on representative densities;
- iOS/iPadOS app icons and launch screen;
- desktop boot, onboarding, huddle loader, app shell, pairing QR center icon,
  repo/link fallbacks, and agent activity indicators;
- web/admin favicon, invite, repository, and dark/light themes;
- reduced motion, contrast, screen-reader names, and transparent safe zones.

Use the repository screenshot workflow for desktop evidence and keep each shot
scoped to a distinct state.

### Quality gates

Run the normal repository gates plus affected platform tests:

```bash
. ./bin/activate-hermit
just ci
just test
just desktop-e2e-smoke
```

Also run mobile analysis/tests and the release/canary, Helm render, icon,
signing, notarization, App Attest, push, and updater checks affected by the
chosen identifier strategy.

## Launch acceptance criteria

- Zorro is the visible product name across supported desktop, mobile, web, and
  admin surfaces.
- Approved Zorro artwork appears at every identified logo/icon/loading/installer
  surface, with no accidental bee or Buzz wordmark remaining.
- Existing Buzz desktop and mobile installations upgrade in place.
- Existing identities, keyring entries, communities, local preferences, nest,
  agent state, and archives remain accessible.
- Every existing `buzz://` link continues to work in released Zorro clients.
- New `zorro://` links work only after all target clients support them.
- Existing CLI, agent, environment, container, Helm, and deployment consumers
  remain operational or have a tested compatibility alias.
- Hosted community URLs do not change tenant identity or cross community
  boundaries.
- Signing, notarization, updater, push, App Attest, and store update paths pass.
- Remaining Buzz strings are explicitly classified and reviewed rather than
  overlooked.
- Current documentation and screenshots are updated in the later documentation
  phase, while historical records remain intact.

## Rollback posture

- Preserve current technical identifiers throughout the first branded release
  so rollback is an ordinary application downgrade where supported.
- Keep old visual assets available for one release branch/tag, not as runtime
  toggles in production.
- Do not delete old keyring entries, storage keys, domains, image coordinates,
  or deep-link handlers during initial rollout.
- For every migration, record a reverse/read-old path before enabling cleanup.
- If Zorro domain or artifact rollout fails, clients must still resolve existing
  Buzz endpoints and release coordinates.
- Treat identity loss, tenant mismatch, invalid signed audiences, broken deep
  links, and stateful infrastructure replacement as release blockers.
