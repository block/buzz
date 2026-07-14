# Changelog

## relay-v0.2.0

- feat: relay invite links (mint + claim + landing page + deep link) ([#1668](https://github.com/block/buzz/pull/1668)) ([`2e529aab7`](https://github.com/block/buzz/commit/2e529aab759a18c1bb81e447f3696fe99db53a27))
- feat(relay): add atomic create-only community provisioning ([#1689](https://github.com/block/buzz/pull/1689)) ([`1de916bdd`](https://github.com/block/buzz/commit/1de916bdd2ad082fb1d33aa1530c483f4a2347ff))
- feat(media): write per-upload-event records for moderation ([#1551](https://github.com/block/buzz/pull/1551)) ([`83fc30b14`](https://github.com/block/buzz/commit/83fc30b14da3ecad5fc9fdd6d71a9bd3ac45c19e))
- fix(desktop): paginate complete channel directory ([#1690](https://github.com/block/buzz/pull/1690)) ([`b41cf3ffc`](https://github.com/block/buzz/commit/b41cf3ffc6d232dc1199f7563bd3c4da8571bfe1))
- [codex] Scope derived runtime state by community ([#1673](https://github.com/block/buzz/pull/1673)) ([`624bd26ce`](https://github.com/block/buzz/commit/624bd26ceae45430f11d263fb73744b648be6314))
- feat(relay): add operator community provisioning ([#1657](https://github.com/block/buzz/pull/1657)) ([`e0f76b0e9`](https://github.com/block/buzz/commit/e0f76b0e9cbd1c84f1ed064f120bc38ab7006d46))
- feat: add deeplink nostr identity binding flow ([#1648](https://github.com/block/buzz/pull/1648)) ([`cecd03142`](https://github.com/block/buzz/commit/cecd031428358d7b3aa0326ccca74649bd928231))
- feat(relay,desktop): canonicalize agent definitions on kind:30175 (Phase 2) ([#1655](https://github.com/block/buzz/pull/1655)) ([`b2c63291f`](https://github.com/block/buzz/commit/b2c63291f1e432951f49c57bd1ec65812870f515))
- Community moderation Phase 1: reports, bans/timeouts, audit, tombstones, relay-DM notices ([#1616](https://github.com/block/buzz/pull/1616)) ([`863aeb79f`](https://github.com/block/buzz/commit/863aeb79f35a43222bd5de3901ab06220948fb9d))
- feat(desktop): canonical <PubKey> component — hover to view/copy full keys, owner "you" labels ([#1589](https://github.com/block/buzz/pull/1589)) ([`777babf39`](https://github.com/block/buzz/commit/777babf3938a2ae7ef97fd12f12e6247e7c18cec))
- feat(nips,relay,acp): NIP-AM durable encrypted agent turn metrics (kind 44200) ([#1441](https://github.com/block/buzz/pull/1441)) ([`71265ca36`](https://github.com/block/buzz/commit/71265ca36105dbf62453a99c998c3f3dd134a304))
- Live thread-summary push: badge counts update on reply ingest ([#1521](https://github.com/block/buzz/pull/1521)) ([`74a30c3de`](https://github.com/block/buzz/commit/74a30c3de8ff61005fecac53eb007e8a616a9229))
- fix: let agent owners delete their agent's messages (relay kind:5 + desktop/mobile UX) ([#1519](https://github.com/block/buzz/pull/1519)) ([`642800548`](https://github.com/block/buzz/commit/6428005487f0690019dc27449a6a52cc29cc6479))
- perf: GIN index for e-tag containment + delta profile fetch (scroll-back ~2.1s/page) ([#1514](https://github.com/block/buzz/pull/1514)) ([`33886e3de`](https://github.com/block/buzz/commit/33886e3dec4130e512d8242207adfe2811a92579))
- GUI read-model overhaul: server-assembled channel windows (Correct™ pagination + relay-signed bounds) ([#1500](https://github.com/block/buzz/pull/1500)) ([`62bb9fe8c`](https://github.com/block/buzz/commit/62bb9fe8c81eee6573c434ffa3227fa96ad9dd4b))
- feat(desktop): repository-first projects with git workflows ([#1471](https://github.com/block/buzz/pull/1471)) ([`8e3c0ee95`](https://github.com/block/buzz/commit/8e3c0ee958af8777ba54fd835de03b0e8eada531))
- feat: per-community workspace icon set by admins, served via NIP-11 ([#1463](https://github.com/block/buzz/pull/1463)) ([`5bfd5ca27`](https://github.com/block/buzz/commit/5bfd5ca2700483498e83224a40a5628a29cf2e9e))
- perf(relay): batch outbound websocket data frames ([#1464](https://github.com/block/buzz/pull/1464)) ([`01b92faa1`](https://github.com/block/buzz/commit/01b92faa156648835f143e84583b8ec3bd7490ab))
- Make reaction ingest atomic ([#1458](https://github.com/block/buzz/pull/1458)) ([`835302cc8`](https://github.com/block/buzz/commit/835302cc829c8a63bf254d3e40156fc446e040f6))
- Serialize fan-out EVENT frames once ([#1459](https://github.com/block/buzz/pull/1459)) ([`3c661fb48`](https://github.com/block/buzz/commit/3c661fb48f81c294b529592d2b2ff874bf96ee96))
- perf(relay): bounded-concurrency multi-filter query execution (S2) ([#1457](https://github.com/block/buzz/pull/1457)) ([`a9e752e25`](https://github.com/block/buzz/commit/a9e752e2540a94d304a51ddeecbf68464ca9ec69))
- fix(read-path): reach complete threads, dense-second timelines, and all people in the GUI ([#1418](https://github.com/block/buzz/pull/1418)) ([`7da936fff`](https://github.com/block/buzz/commit/7da936fff82a9a956f338c690e9605888725ea3b))
- E1+E3: reduce relay ingest/fan-out DB round trips; ack p99 −7–16%, fd p99 −6–28%, p999 tails −29–53% vs PR #1453 tip ([#1454](https://github.com/block/buzz/pull/1454)) ([`a504ad619`](https://github.com/block/buzz/commit/a504ad6197558575c0db7b9f53806d7337e0c64f))
- perf(relay): defer post-commit dispatch and avoid verify clone ([#1453](https://github.com/block/buzz/pull/1453)) ([`7bd3760c8`](https://github.com/block/buzz/commit/7bd3760c82a6d640af199ed2301525877e629ced))
- fix(relay): include git hook tools in runtime image ([#1326](https://github.com/block/buzz/pull/1326)) ([`88c089d3b`](https://github.com/block/buzz/commit/88c089d3b652bc952adbe8b32a6fc585121c982f))
- fix(relay): remove media bearer-token auth ([#1444](https://github.com/block/buzz/pull/1444)) ([`0701f47f4`](https://github.com/block/buzz/commit/0701f47f4a31a904ebcd9f360cbd6aadaff9d784))
- feat(relay): add OpenTelemetry tracing, keep Prometheus metrics ([#1398](https://github.com/block/buzz/pull/1398)) ([`b1d9d955d`](https://github.com/block/buzz/commit/b1d9d955de83538c231a3034bf190af6df03070d))
- feat(git): move repo-name registry to Postgres + relax RWM chart gate (HA relay) ([#1432](https://github.com/block/buzz/pull/1432)) ([`e5aa4a213`](https://github.com/block/buzz/commit/e5aa4a21327438c02fb25baea4d0849a498c9059))
- fix(relay): enable Redis TLS for rediss:// (ElastiCache) ([#1417](https://github.com/block/buzz/pull/1417)) ([`3292b502a`](https://github.com/block/buzz/commit/3292b502aad44a5f849296d7bf28429bac272fb7))
- feat(relay): allow agent owners to edit/manage agent-owned content ([#1403](https://github.com/block/buzz/pull/1403)) ([`0042c8e10`](https://github.com/block/buzz/commit/0042c8e106d952f408cf4afca052f7053a7c967e))
- fix(media): support IRSA/credential-chain S3 auth and configurable signing region ([#1406](https://github.com/block/buzz/pull/1406)) ([`06ef533ec`](https://github.com/block/buzz/commit/06ef533ec7fec6cf7366f52d3b9fe2f83011bf24))
- Harden relay attack surfaces ([#1369](https://github.com/block/buzz/pull/1369)) ([`29368cf17`](https://github.com/block/buzz/commit/29368cf17b7d5924fe571512b2194e3f48b21a16))
- Multi-tenant Buzz relay: community_id as a server-resolved key (comprehensive rewrite) ([#1321](https://github.com/block/buzz/pull/1321)) ([`14fba21e5`](https://github.com/block/buzz/commit/14fba21e57b8d671ebbea473226be52a5f2ae636))
- Fix cross-pod membership notification fanout ([#1291](https://github.com/block/buzz/pull/1291)) ([`e1c51d71d`](https://github.com/block/buzz/commit/e1c51d71d48f9ad0a08c3e7e0af35929a0234ea1))
- Add NIP-34 git pull request CLI support ([#1279](https://github.com/block/buzz/pull/1279)) ([`5fef9b727`](https://github.com/block/buzz/commit/5fef9b727fb8856d45840ccc97d8ec820a48f6e0))
- chore: remove LLM-slop comments across the codebase ([#1277](https://github.com/block/buzz/pull/1277)) ([`73cc31cc5`](https://github.com/block/buzz/commit/73cc31cc528318debedd38e85d67b79d1feb55e8))


## relay-v0.1.1

- Initial release
