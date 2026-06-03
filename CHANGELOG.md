# Changelog

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
