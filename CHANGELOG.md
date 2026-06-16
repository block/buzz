# Changelog

## v0.3.24

de24b90aa feat(desktop): refine thread-unread badge to two-token form (#1069)
fd2553726 fix(buzz): prevent reconnect storms from reaped ephemeral channels (#1071)
5a651632d fix(buzz-acp): trim oversized observer frames to fit instead of dropping (#1072)
07efae7b5 perf(ci): speed up PR CI wall clock and local dev builds (#1028)
56a5ac279 chore(deps): update react monorepo (#1048)
9f99e62d4 Polish desktop visual details (#1067)
e3736f08b ci: use running postgres for pgschema desired-state planning (#1070)
2d26db6d8 fix(desktop): anchor active-turn badge to skew-corrected agent start (#1068)
ba776b995 feat(desktop): add configurable transport reconnect hook (#1059)
2300248d3 Add automatic database migrations (#988)
f8715612a Add composer spoiler formatting (#1055)
2a2c1c800 feat(desktop): in-channel and in-thread unread indicators (#1008)
cbc754cff perf(timeline): gate heavy message render behind useDeferredValue (#1022)
116486592 Add animated profile avatars (#1031)
89ae31d20 Polish direct message and members modals (#1054)
5234fc816 Polish huddles UI (#1041)
424ea7025 Fix video review comments in threads (#1056)
81296d976 Polish message reaction tray (#1002)
c30d7274c Refine app loading skeletons (#1001)
ee34ca818 Polish channel modal forms (#1000)
19656fff4 Normalize desktop icon sizing (#999)
7a2e35521 Add shared skeleton loader primitives (#998)
9ee5aeebd chore(scripts): update post-screenshots repo name to block/buzz (#1042)
b2ad3074b docs: fix stale sprout repo references in RELEASING.md (#1043)


## v0.3.23

9b410325 fix(release): publish manifest from successful platforms (#1039)


## v0.3.22

fix(release): publish rolling updater manifest for automated release tags


## v0.3.21

4d19a590 fix(release): use signed NSIS installer for updates (#1036)
fa1cade3 handoff: pass full session history to summarizer (#1033)
6e4c8680 feat(emoji): latest-set-wins union for custom emoji across desktop, mobile, and CLI (#989)
5f8ab33b Fix relay NIP-11 software URL (#1030)
5c2f46e6 fix(desktop): make Windows release compile cleanly (#1029)
6caa359d Add production Docker Compose bundle (#985)
a32681fd feat(profile): show active turn badges on agent profile panel and popover (#1026)


## v0.3.20

c7dd4295b fix(release): resolve Windows sidecar path and Linux AppImage updater format (#1024)


## v0.3.19

faf00724f fix(release): ignore prerelease tags in changelog generation (#1021)
b8c0556e7 fix: repair main build after cross-PR merge skew (#1020)
87e45c65b feat(agents): show per-turn duration and prune dead turns within ~25s of host crash (#1017)
2fef8d664 fix(release): replace hermit with native tool setup on Windows job (#1018)
6db90514b feat(acp): surface error-class outcomes to the activity feed only, never the channel (#1010)
563f68434 fix(desktop): migrate Sprout workspace storage (#1016)
5a8cc79c6 feat(auth): force token refresh on rejected token (401/403), never the browser (#1015)
59a7e5da8 fix(release): mark prerelease versions so they do not become latest (#1013)
f08588245 feat(acp): implement systemPrompt with protocol version gating (#981)
d07c8216c fix(release): update repository name check from block/sprout to block/buzz (#1012)
de641fce5 feat(release): all-OS desktop builds + universal auto-update manifest (#1011)
8c9211ffc Add relay disconnect UX: friendly errors, reconnect, cached identity (#1004)
7983bf675 feat(agents): add active turn indicators to Agents Menu (#1005)
39d9aa826 ci: add fork guards to docker, release, and auto-tag workflows (#1007)
43d1ce353 docs(nip-rs): add optional thread read context scheme (#1006)
12433077a fix(huddle): Pocket TTS quality overhaul — reference parity + cross-message pipelining (#997)
00dc4915d Add manual ACP session rotation command (#932)
df8896f13 fix(desktop): heal stale persona_team_dir paths in release builds (#1003)
1fa63bada ci(docker): publish public ghcr.io/block/buzz image (native multi-arch) (#986)
84f499cb6 fix(buzz-agent): cap tool-result text at 50 KiB with middle elision (#952)
2846a96ed feat(huddle): sentence-at-a-time voice-mode guidelines for lower TTS latency (#996)
a1c28f487 Shard desktop Playwright CI jobs (#992)


## v0.3.18

05fc69b8408 Video Player Improvements  (#993)
d9ce0943edb Improve first-run welcome setup (#970)
50986406ffa fix(release): use legacy updater key secret (#991)
ea5a0a9b405 Replace built-in personas with Fizz (#987)
6541765416b docs(buzz-acp): rewrite Communication Patterns for mention accuracy and threading clarity (#982)
a101fd6ad38 chore(justfile): build git-credential-nostr in dev and staging recipes (#980)
824c55114ef Fix Buzz command migration for saved agents (#979)
63738139bac fix(desktop): resolve effective model and prompt from persona in display path (#972)
1bb8b8d547a docs: clean up remaining Buzz references (#977)


## v0.3.17

79bcee55cb7 docs: finish Buzz rename cleanup (#974)
6f3733d43b4 fix(desktop): let channel members bypass mention agent gate (#965)
8f580f308cc Rename desktop app to Buzz (#960)
dcb2639b355 feat(desktop): open profile panel from MembersSidebar rows (#962)
4e4dc723e4c feat(desktop): per-event notification sounds and alert controls (#968)
1ca16c898c7 fix(desktop): make header chrome zoom-correct and tidy split-pane (#941)
8c8312932af fix(desktop): rename SPROUT_ env vars to BUZZ_ for child agent processes (#971)
31b0665cff7 fix(justfile): complete buzz rename in dev and staging recipes (#966)
d99ad131f17 refactor: rename sprout backend to buzz (#958)
53e3f094858 fix(desktop): reap orphaned agent processes across instances (#954)
c5a54dcc390 Rename web app to Buzz (#959)
510009c11db fix(desktop): allow restarting saved relay-mesh agents from the UI (#956)
60c8c5036a2 feat(acp): agent timeout resilience — idle margin, tool-call reset, death notices, keepalive (#935)
c63e018b05b Rename mobile app to Buzz (#955)
929cc8861ee fix(desktop): repair team-persona mismatch and deduplicate legacy imports (#949)
b792aa4704a fix(desktop): populate last_message_at in channel browser (#951)
e5f0c32648b Kit/circular avatars (#927)
dbe973dacd0 fix(relay): accept mesh signaling kinds (24620/24621) via POST /events (#946)
f49cdcdd300 feat(sprout-dev-mcp): add read_file tool and replace_all to str_replace (#928)


## v0.3.16

34c8bdab1 fix(desktop): land live presence updates for not-yet-cached pubkeys (#947)
5c0af0bc9 fix(release): make `just release` idempotent for re-runs (#948)
e9cd1c392 Improve mentions for agents + people  (#942)
384eb6cba feat: agent memory viewer (read-only) in profile panel (#917)
2dc466fe0 Fix channel visibility controls (#940)
3e56331e9 fix(delete): make agent-deleted messages disappear from desktop UI immediately (#918)
e08937cdd Fix emoji message rendering (#938)
ba2fdbf69 refactor(desktop): consolidate packs into teams (#852)
384a34ec3 Update README.md wording
fe14daa5d Fix post-compact handoff context for OpenAI providers (#931)


## v0.3.15

877048d68fb fix: persona is source of truth at spawn + thread-depth conventions (#930)
73cd8d082d4 fix: skip avatar reconciliation for legacy agent records (#933)
165b9f7a5f5 feat(desktop): add nest commit identity guidance with human sign-off (#929)
9a98e60fc29 feat: provider/model selection for personas and runtime-aware env injection (#794)
762a45969a6 fix: reconcile agent profile on startup when relay publish was missed (#921)
5d927aba0c4 Revamp first-run onboarding (#924)
f3626501952 Update setup loading screen (#926)
c38301bca5a fix(dm): keep hidden DMs hidden across refetch via relay-signed visibility snapshot (NIP-DV) (#857)
4dfae61f242 Maximize desktop window on launch (#925)
ae430d4dd95 feat: preview features (experiments settings UI) (#888)
a357e220d82 fix(updater): send no-cache header on update check to avoid stale manifest (#922)
56230c1442f fix(desktop): refresh channel state after unarchive (#923)
7dd5b34536b Add channel visibility & ephemeral TTL controls to manage sidebar (#911)
4b78fe3bea2 ci(release): add Intel macOS (x86_64) DMG as a release target (#748)
421593062f2 mesh: Rust-owned coordinator — fix saved-agent reconnect flakiness + DRY the start path (#879)


## v0.3.14

bfafdd46b29 fix(sdk): resolve multi-word display names and add NIP-27 nostr:npub mention extraction (#905)
15f610dcd5c fix(desktop): re-enable mcp_command reconciliation and harden spawn site (#909)
da80c7340f3 Fix desktop DM and sidebar UI polish (#908)
dd08f988dec Animate reaction counts (#904)
10b6674bd79 Mobile custom emoji + settings redesign (#906)
732e23dd5c3 Renew TTL when unarchiving ephemeral channels (#902)


## v0.3.13

ecca5e77e4e Collapse channel header actions (#901)
4ec7f8125e8 sprout-agent: make Databricks defaults env-only (#868)
b384354e2bd Restyle settings sections (#894)
fdcbb696fe0 Add emoji reaction particles (#890)
32039b9a25b Move settings into the app shell (#893)
45f3dfe5ba6 Tune chat text sizing (#891)
29f6ccf9e9c Style channel header navigation (#889)
2ebe5517410 fix: rename missed known_acp_provider_exact → known_acp_runtime_exact (#900)
97bdb79ded5 chore(deps): update radix-ui-primitives monorepo (#898)
4a93100e199 chore(deps): update actions/checkout digest to df4cb1c (#897)
0a6067ca1be refactor: rename ACP "provider" to "runtime" across the codebase (#783)
056b87d3da4 Unify avatar radius (#892)


## v0.3.12

1b7b6978fc8 Show hover cards for inline message emoji (#885)
5268fac2d84 Fix monotonic read-state merges (#884)
0a4783c6f8a Refine sidebar behavior and borders (#869)
5d7c7489698 fix(presence): clear on disconnect, fix heartbeat/TTL, drop broken REST path (#877)
ef98ae942a5 fix(cli): publish ephemeral events over WebSocket via sprout-ws-client (#876)
2f50011bdd2 docs(sprout-acp): add communication discipline rules to base prompt + deprecate --mention flag (#883)
5c2476a71e4 Polish thread summaries and reactions (#881)
7129cd6f23e feat(cli): add emoji export and import subcommands (#882)
b84f8e6a010 Polish message row hover states (#880)
bc53008676f Improve emoji naming and custom emoji UX (#878)
581c7e95a9b docs: add ecosystem section to CONTRIBUTING.md, fix stale release info (#873)
031152221bd fix(relay): wire custom filter fields through HTTP bridge (#864)
f1c672fea53 chore: deprecate sprout-mcp — fill CLI gaps, remove crate and all references (#850)
5bdac0566fb Fix custom emoji status in profile popover (#874)
b295f51c904 fix(agent): gate handoff on provider token usage, not byte estimate (#821)
cdb7bc27e11 docs: add VISION_MESH.md — the compute-commons vision (#867)
d4bb7f66e0d fix(desktop): simplify profile popover header (#853)
ccff5464a41 fix(desktop): remove thread comment hover outline (#861)
ad7ab482eb1 feat(desktop): always show channel section search/add buttons (#856)


## v0.3.11

269b35e8de7 fix(mobile+desktop): cross-device read state sync + diagnostic logging (#843)
3ddfe5fcc25 feat(mobile): star channels (Slack-style favorites) (#863)
36d7dbd7cae feat: desktop-screenshot skill to stop agents uploading relay media to PRs (#862)
c10b4f8f5c6 feat(desktop): star channels (Slack-style favorites) (#860)
f748f71268e fix(desktop): handle symlinked persona pack directories (#859)
1fe7bf28725 feat: channel muting for desktop and mobile (#838)
4ead7de4630 feat(acp): default SPROUT_ACP_MEMORY to on (#854)
759e5cd9235 fix(desktop): eliminate image-hover layout jump in messages (#813)


## v0.3.10

34ac3ba1d18 fix(desktop): harden relay mesh connect p-tag (#834)
b3aefae152e fix(desktop): scroll activity panel to bottom on open (#848)
a13691b6207 Polish desktop profile menu interactions (#836)
0d9b8148f86 fix(desktop): outline thread hover targets (#845)
b3be9ecba70 fix(desktop): keep message actions hover-only (#844)
db46c425463 fix(desktop): let inbox composer fill available width (#841)
2a0572c0d8e fix: use immutable commit-SHA URLs in screenshot PR comments (#842)
3b78dc5690b feat(mobile+desktop): two-tier Slack-style app icon badge (#802)
0c225f4d7d3 chore: simplify file-size check to a flat 1000-line limit (#839)
d8b602a3595 fix(desktop): robust emoji picker — unify picker + fix custom emoji in editing, status, reactions (#837)
06bc67fd342 feat(desktop): reusable screenshot workflow for agents (#826)
9f0c22a43a6 desktop(mesh-llm): let a serving node route a different model (#833)


## v0.3.9

82ae85f79a9 fix: native arbitrary-file download + image context-menu flash (#830)
7797ae77f64 fix(desktop): custom emoji reaction rendering + picker autofocus (#831)
33cfc852932 Mesh-LLM v1: relay-gated direct-iroh inference between users (WAN) (#822)


## v0.3.8



## v0.3.7

2349421304d feat: custom emoji — user-owned NIP-30 sets with a client-side union (#816)
7a12e50518e Install sprout-cli skill at repo root + fix desktop clippy (#818)
2ea3fd88d23 fix(desktop): use public re-export path for ensure_client_node_for_model (#824)
fb514c8918d refactor(desktop): feature-gate mesh-llm-sdk behind optional Cargo feature (#823)
b72eee365f0 fix(desktop): align workflow read/save commands to the frontend contract (#820)
5b572d6f5e9 fix(desktop): disable mesh-llm auto-build to prevent git config corruption (#819)
192388a3cbf fix(desktop): clear clippy lints in agents/mesh_llm commands (#817)
41a3fc1589b fix(desktop): let channel members add members and bots without admin (#815)
a25ca5d1bfb Desktop #806 follow-ups: panel/inbox fixes + top-bar backdrop (#814)
5bed17a1173 Fix desktop right-side panel chrome overlap (#806)
ede8ddb425c Sprout × mesh-llm: in-process mesh node (serve/consume) + relay admission (#798)
6481428e2b9 fix(desktop): resolve flaky integration tests via project-level assertion timeout (#812)


## v0.3.6

5cbedb180af feat(mobile): add channel sections with relay sync (#800)
753d0fe264d feat(desktop): sync channel sections across devices via Nostr (#792)
2b052eb465f feat(media): support arbitrary file types with download cards (#810)
247ac523915 feat(desktop): add user-defined channel sections to sidebar (#789)
d810608d859 feat(desktop): keyboard shortcuts — ⌘⇧N new channel + ↑-to-edit last message (#809)
39911e42859 fix(desktop): scope agent sweep to the owning app instance (#808)
f2c266bac23 fix(desktop): route notification clicks to thread context (#790)
33e37de6e37 chore(deps): update all non-major dependencies (#804)
5670ffc6a6e fix(deps): re-pin isomorphic-git patch to 1.38.3 (#807)
033d92f103e chore(deps): update dependency @tanstack/react-query to v5.100.14 (#805)
bc23620fcfa Fix desktop glass chrome and inbox previews (#793)
9b9cf461278 refactor(just): slim down mobile-dev to just run Flutter (#801)
2a03851527f refactor: consolidate infra management into justfile + add mobile-dev (#797)


## v0.3.5

b820420909d feat(mobile): Pulse polish — flat feed, compose page, shared filter chips, like + accent fixes (#796)
10f37e4cab2 feat(desktop): add standalone Playwright screenshot helper (#795)
f34a21d32ff feat(sprout-agent): load AGENTS.md and SKILL.md into system prompt (#762)
85861f33fe0 feat: add code block support to message composer (#788)
7beb0f8e685 fix(desktop): reap orphaned agent processes on shutdown and restart (#787)


## v0.3.4

d77b111b153 Update desktop navigation chrome and search (#779)
5ee2cd05173 feat(desktop): reload webview on Cmd/Ctrl+R (#785)
fa7febe40f5 fix(desktop): sync persona pack directory across worktree instances (#782)


## v0.3.3

c761a76ff2d fix(release): sync release tags during preflight (#780)
3f3ec64791b feat(desktop): thread-aware notifications with mutable follow/mute controls (#761)
03e678cfbca fix(desktop): improve model picker message and runtime dropdown clarity (#778)
9db8f6ccb88 desktop: float unread indicator + fix sidebar scroll jump (#777)
3fbee555f06 chore(hooks): standardize check/fix convention with auto-fix pre-commit (#776)
0f89ad16925 web: clickable repo tree + per-file blob viewer (#773)
61297ac80a3 fix(agent): keep parallel tool-result messages contiguous on OpenAI Chat (Databricks image fix) (#770)
5f2423c231e fix(release): fetch tags so changelog tracks versions correctly (#775)


## v0.3.2

1218572f feat(mobile): add Pulse social feed tab (#772)
fec6a683 feat(sidebar): add More unread floating buttons (#771)
8eedec74 chore: improve markdown spacing (#766)
835a44aa fix: prevent inline links rendering in 2-column grid layout (#767)

## v0.3.1

4222a758 [codex] Default release command to patch bump (#768)
30654e95 Polish desktop Pulse and Home views (#764)

## v0.3.0

Initial release on the automated pipeline. Unifies OSS and internal version numbering above both v0.0.21 (OSS) and v0.2.38 (internal).
