# Changelog

## v0.3.30

- fix(desktop): collapse mark-read/unread menu into one toggling item ([#1188](https://github.com/block/buzz/pull/1188)) ([`ce994df74`](https://github.com/block/buzz/commit/ce994df74e60cf43b2fb0b97ea9989aacd47650e))
- fix(channels): replace thread-unread badge frontier with per-message read markers ([#1178](https://github.com/block/buzz/pull/1178)) ([`28db41dfd`](https://github.com/block/buzz/commit/28db41dfd383f3f7acd57011bd98661a08e7bfe4))
- fix(relay): resubscribe agents when an archived channel is restored ([#1187](https://github.com/block/buzz/pull/1187)) ([`6780ea21e`](https://github.com/block/buzz/commit/6780ea21e44a525aa73455c4ae79702564f783f2))


## v0.3.29

- build: make mesh-llm opt-in for dev/staging to speed up iteration ([#1183](https://github.com/block/buzz/pull/1183)) ([`ce526106`](https://github.com/block/buzz/commit/ce5261067d2b970ecca7e0d6c24479fcd2ccad43))
- feat(desktop): default temporary channels to 7-day expiry ([#1182](https://github.com/block/buzz/pull/1182)) ([`4b518fc4`](https://github.com/block/buzz/commit/4b518fc4b3ec6c55eeb4d4b203057d55d8ba0c56))
- fix(desktop): render lightbox image sharply at full resolution ([#1181](https://github.com/block/buzz/pull/1181)) ([`ff727af9`](https://github.com/block/buzz/commit/ff727af926ce5a7a840b042d6cff4b5a40a81db5))
- fix(desktop): keep thread guides behind the reply composer ([#1179](https://github.com/block/buzz/pull/1179)) ([`781d1acf`](https://github.com/block/buzz/commit/781d1acf8a412df882fbd49ae1dd2f31a1a3096f))
- Sort direct messages alphabetically ([#1180](https://github.com/block/buzz/pull/1180)) ([`04df2c8d`](https://github.com/block/buzz/commit/04df2c8dc6ab055a5a9360f4a492715af6d28dc1))
- Add composer selection formatting tray ([#1133](https://github.com/block/buzz/pull/1133)) ([`7dcf22d5`](https://github.com/block/buzz/commit/7dcf22d50eb4c7503248caa95398f889e6290839))
- fix(desktop): honor per-agent relay override, default to workspace relay ([#1131](https://github.com/block/buzz/pull/1131)) ([`ea97e219`](https://github.com/block/buzz/commit/ea97e219895e396708cfc626d64b21f864a2674e))
- [codex] Stabilize desktop E2E flakes ([#1174](https://github.com/block/buzz/pull/1174)) ([`141f7118`](https://github.com/block/buzz/commit/141f7118ae6aedd910966651c53d92f9c6f71702))
- feat(desktop): show reminder author + source and click-to-navigate in inbox ([#1176](https://github.com/block/buzz/pull/1176)) ([`1186ea48`](https://github.com/block/buzz/commit/1186ea48b99cf2d2cd789137b3e907cee5d6e16c))
- Improve inbox thread updates ([#1114](https://github.com/block/buzz/pull/1114)) ([`de3f21ab`](https://github.com/block/buzz/commit/de3f21abcd60985c47f43c34d963a68300018f0b))
- docs(readme): mark git hosting as shipped in status table ([#1175](https://github.com/block/buzz/pull/1175)) ([`26fdfb67`](https://github.com/block/buzz/commit/26fdfb674c387fca20637ee4c2169f20fb98e50b))
- feat(windows): bundle full Git-for-Windows toolchain for the shell tool ([#1145](https://github.com/block/buzz/pull/1145)) ([`592508ad`](https://github.com/block/buzz/commit/592508adddd889d1b2bbfd8b5384fc2b7107c35c))
- fix(buzz-dev-mcp): resolve MSYS-absolute paths in the Windows file tools ([#1169](https://github.com/block/buzz/pull/1169)) ([`ec5cfa37`](https://github.com/block/buzz/commit/ec5cfa3796d88b6ea92c620817163e5c10fad8b3))


## v0.3.28

- Improve thread branch display ([#1166](https://github.com/block/buzz/pull/1166)) ([`342a251a`](https://github.com/block/buzz/commit/342a251a9199b55ffc0e19cb50ee7e09a8ec051f))
- Copy channel name from header ([#1157](https://github.com/block/buzz/pull/1157)) ([`a96d173e`](https://github.com/block/buzz/commit/a96d173e996da82a1e59fbdd60081ead01a0e869))
- docs: add product screenshots to README ([#1168](https://github.com/block/buzz/pull/1168)) ([`ff976e69`](https://github.com/block/buzz/commit/ff976e690e6488b0ea90336d5d724ccb097548ac))
- Use accent color for video timecodes ([#1167](https://github.com/block/buzz/pull/1167)) ([`c243a013`](https://github.com/block/buzz/commit/c243a01362e03e5436e22618fe60f89e71fed1b2))
- Add video playback speed control ([#1164](https://github.com/block/buzz/pull/1164)) ([`f218b815`](https://github.com/block/buzz/commit/f218b8158ef426b26013f0508668f3fee000b368))
- Update README.md ([`fee4b1f5`](https://github.com/block/buzz/commit/fee4b1f556b97e06dfb5dda34859ddf6250e13ea))


## v0.3.27

- chore: remove block team id from Xcode ([#1156](https://github.com/block/buzz/pull/1156)) ([`67fcf41b`](https://github.com/block/buzz/commit/67fcf41b53e081d1bed950257a3372683905a81c))
- fix(desktop): keep channel chrome clear of window controls ([#1159](https://github.com/block/buzz/pull/1159)) ([`fa10871d`](https://github.com/block/buzz/commit/fa10871df61f4426441e887839772e49f4e03196))
- desktop: polish sidebar search dialog ([#1155](https://github.com/block/buzz/pull/1155)) ([`5f79830e`](https://github.com/block/buzz/commit/5f79830e191102b4884487474ea70e1e522788a2))
- desktop: fix scrollback paging and visible history depth ([#1153](https://github.com/block/buzz/pull/1153)) ([`6cca7599`](https://github.com/block/buzz/commit/6cca75993be9b61c9a50159130492c183fcd2304))
- Fix ACP activity tool title detection ([#1149](https://github.com/block/buzz/pull/1149)) ([`4c7df9b0`](https://github.com/block/buzz/commit/4c7df9b0b3ea6884d0d9f59d7d1b244542885495))
- fix(desktop): clear channel unread badges from thread reads ([#1148](https://github.com/block/buzz/pull/1148)) ([`7c3e411f`](https://github.com/block/buzz/commit/7c3e411fa8df14ef706bfd657899de6ec1f976ff))
- desktop: move global search into sidebar header and remove vestigial top band ([#1150](https://github.com/block/buzz/pull/1150)) ([`c488d845`](https://github.com/block/buzz/commit/c488d8452c50f882a28b3f09224f3c6a30fb1b47))
- Update README.md ([`7c67dfa6`](https://github.com/block/buzz/commit/7c67dfa66d38c1f25d7fab185ea1b2584bb82898))
- Hide channel intro actions until history start ([#1151](https://github.com/block/buzz/pull/1151)) ([`7aa6daa4`](https://github.com/block/buzz/commit/7aa6daa4967c88dcda45560f6d4c5d2a0a2a63da))
- fix(desktop): remount timeline scroll node per channel ([#1147](https://github.com/block/buzz/pull/1147)) ([`0c5dbf34`](https://github.com/block/buzz/commit/0c5dbf3452802a0370c1bdb445efa602c98c4148))
- Render custom emoji in the inbox; DRY up emoji derivation ([#1146](https://github.com/block/buzz/pull/1146)) ([`39626197`](https://github.com/block/buzz/commit/39626197c0c80ae29dbb9814b6f3b6a603b174d0))
- desktop: open thread pane optimistically ([#1143](https://github.com/block/buzz/pull/1143)) ([`663b554c`](https://github.com/block/buzz/commit/663b554c7c42371aac66fa2d6cb9524280f88600))
- fix(desktop): keep channel header search/add icons always visible ([#1144](https://github.com/block/buzz/pull/1144)) ([`9c685282`](https://github.com/block/buzz/commit/9c6852824eecceecb5d504ce61a4906dd6a2aab9))
- fix(desktop): show skeleton during channel switch ([#1141](https://github.com/block/buzz/pull/1141)) ([`d1c3574b`](https://github.com/block/buzz/commit/d1c3574ba73b2febb8ee8306b58fddffce6001bf))
- Avoid duplicate thread event fetches ([#1129](https://github.com/block/buzz/pull/1129)) ([`959f2598`](https://github.com/block/buzz/commit/959f25982bbd135182c7c9f0d9c810a0324f1b2e))
- desktop: restore channel browse button ([#1140](https://github.com/block/buzz/pull/1140)) ([`cc8958f5`](https://github.com/block/buzz/commit/cc8958f5cc0421c1cabe7cee882da898fc09e1f0))
- desktop: make date dividers pill shaped ([#1139](https://github.com/block/buzz/pull/1139)) ([`bee41597`](https://github.com/block/buzz/commit/bee41597157eec63ec631caf3b1b6fb6e114e268))
- fix(desktop): align sidebar unread pills ([#1138](https://github.com/block/buzz/pull/1138)) ([`59754218`](https://github.com/block/buzz/commit/5975421889f2cdf31f353deb0b401ddfc78b86c4))
- Fix display names flashing to pubkeys when loading older messages ([#1137](https://github.com/block/buzz/pull/1137)) ([`c448f1c7`](https://github.com/block/buzz/commit/c448f1c72966b045f7d9b84560b3c1d04ca8b72c))
- desktop: keep sidebar unread pill below top chrome strip ([#1136](https://github.com/block/buzz/pull/1136)) ([`aeafb0b0`](https://github.com/block/buzz/commit/aeafb0b0b2623789a47be78cbd35782866fb00b3))
- fix(desktop): restore sticky date divider handoff ([#1135](https://github.com/block/buzz/pull/1135)) ([`3e9d7ff4`](https://github.com/block/buzz/commit/3e9d7ff4d6a152c944cf45ecfa70b641eac0f393))
- desktop: drop redundant post-subscribe history refetch ([#1130](https://github.com/block/buzz/pull/1130)) ([`b2bf471e`](https://github.com/block/buzz/commit/b2bf471ea581e768647f3f3ad055bffbf7803f08))


## v0.3.26

- ci: cache Flutter pub packages in the Mobile job ([#1128](https://github.com/block/buzz/pull/1128)) ([`ceb5eff53`](https://github.com/block/buzz/commit/ceb5eff53717a18afb9ac6cd3f3a93f01394b5bc))
- Delete RESEARCH directory ([`a1d5570d3`](https://github.com/block/buzz/commit/a1d5570d3136c2c52823f512a5c59429f8995ca0))
- feat(cli): support ephemeral channels via --ttl on create/update ([#1126](https://github.com/block/buzz/pull/1126)) ([`4f671a255`](https://github.com/block/buzz/commit/4f671a255cb526520151596b01dcf4a50ab2242d))
- fix(justfile): suppress grep exit code in release changelog formatter ([`d40c86626`](https://github.com/block/buzz/commit/d40c86626a3c2d10eaa647f0e4664b32ef9060a5))
- fix(desktop): single-owner anchored scroll for dynamic loading ([#1115](https://github.com/block/buzz/pull/1115)) ([`047db4214`](https://github.com/block/buzz/commit/047db4214e6ce79a615b8842e83f9f03c8e10f55))
- fix(buzz-dev-mcp): resolve non-WSL bash so the MCP shell works on Windows ([#1119](https://github.com/block/buzz/pull/1119)) ([`4d2eb52a6`](https://github.com/block/buzz/commit/4d2eb52a6c89fe71f37c2c5abf3fed9e95d45977))
- ci: key main-push concurrency group on SHA to stop run eviction ([#1124](https://github.com/block/buzz/pull/1124)) ([`e99105edc`](https://github.com/block/buzz/commit/e99105edc47bf36facbadbc94aeea99c7f15dd38))
- fix(desktop): seed up agent JSON files in sync_shared_agent_data ([#1121](https://github.com/block/buzz/pull/1121)) ([`065625bb4`](https://github.com/block/buzz/commit/065625bb4f8dc49e1d3d56698c637383f9f82eef))
- fix(acp): emit per-section prompt blocks so observer counts every section ([#1122](https://github.com/block/buzz/pull/1122)) ([`a1cf1db67`](https://github.com/block/buzz/commit/a1cf1db67faf9728fa1301ece5ef838ff2b2d906))
- refactor(justfile): rename, relay auto-start, bootstrap, DRY cleanup ([#1117](https://github.com/block/buzz/pull/1117)) ([`1dc4fb5da`](https://github.com/block/buzz/commit/1dc4fb5daf048e5e44af82450b067175c75814c2))
- fix(desktop): keep active-turn badges through transient relay drops ([#1120](https://github.com/block/buzz/pull/1120)) ([`cf122fcf1`](https://github.com/block/buzz/commit/cf122fcf1406378e5de2e652bd3f66ce5516f56b))
- Polish desktop sidebar navigation ([#1107](https://github.com/block/buzz/pull/1107)) ([`7072281f6`](https://github.com/block/buzz/commit/7072281f63de5005b9046af58ef3fb38b151ee09))
- fix(desktop): collapse thread-unread badge on thread-open ([#1118](https://github.com/block/buzz/pull/1118)) ([`0176d5f31`](https://github.com/block/buzz/commit/0176d5f31ec327641444fc89edb8ee1f04638578))
- feat(desktop): unify unread pills into one shared UnreadPill component ([#1111](https://github.com/block/buzz/pull/1111)) ([`826945e1b`](https://github.com/block/buzz/commit/826945e1bbee2b93a2bc5ec13c2d1c3241bb7754))
- fix(cli): use relay workflow id on create ([#872](https://github.com/block/buzz/pull/872)) ([`762ca43d2`](https://github.com/block/buzz/commit/762ca43d26be8f5376f96a1376ed7eb6f213e411))
- Fold agent core memory into the session system prompt ([#1112](https://github.com/block/buzz/pull/1112)) ([`633544788`](https://github.com/block/buzz/commit/6335447887f2d8cc8ea7fa2468f37a287a08e224))
- feat(cli): add patches and issues commands for NIP-34 git collaboration ([#1073](https://github.com/block/buzz/pull/1073)) ([`cd8292fe9`](https://github.com/block/buzz/commit/cd8292fe9e7941b3b25510c604491041a723cb7a))
- fix(desktop): stop random timeline message loss + page reconnect replay ([#1105](https://github.com/block/buzz/pull/1105)) ([`0181603eb`](https://github.com/block/buzz/commit/0181603eba2a1a632467105d00fd41ddb96c526a))
- Update README.md ([`c338d8840`](https://github.com/block/buzz/commit/c338d8840cb21ad27475b1d4b29f6fec539ca663))
- fix(desktop): keep thread replies from scrolling channel ([#1109](https://github.com/block/buzz/pull/1109)) ([`d24eac61a`](https://github.com/block/buzz/commit/d24eac61a86f0da63e12e2758863b8364b67320d))
- fix(buzz-acp): accept siblings under allowlist author gate ([#1108](https://github.com/block/buzz/pull/1108)) ([`dd9ce0902`](https://github.com/block/buzz/commit/dd9ce090209c282921c7c0e821795b12ac69796f))
- feat(deploy): add production Helm chart for Buzz ([#990](https://github.com/block/buzz/pull/990)) ([`629fb57bf`](https://github.com/block/buzz/commit/629fb57bf0802f1b57eaa3068fc6eaad85b9d38f))
- fix(desktop): keep MembersSidebar input usable while an add is in flight ([#1106](https://github.com/block/buzz/pull/1106)) ([`aadbe6735`](https://github.com/block/buzz/commit/aadbe67354f407a31c913de87b3e3d698fc3dddf))


## v0.3.25

- fix(desktop): stop dimming deferred message lists ([#1104](https://github.com/block/buzz/pull/1104)) ([`718b596e5`](https://github.com/block/buzz/commit/718b596e5b696dbe439993c40a16ba39dd862828))
- Smooth channel loading: single-surface timeline state machine ([#1099](https://github.com/block/buzz/pull/1099)) ([`d47572209`](https://github.com/block/buzz/commit/d47572209dd26ee11cb7d77c85d420fb11947ce8))
- feat: surface base + persona system prompts in observer feed ([#1103](https://github.com/block/buzz/pull/1103)) ([`5004758fb`](https://github.com/block/buzz/commit/5004758fbfda8ea9c014174b43464b9835952090))
- ci: move reminder e2e to a dedicated backend-integration job ([#1098](https://github.com/block/buzz/pull/1098)) ([`466bb993c`](https://github.com/block/buzz/commit/466bb993ca5a5bbf99db47ec31356b1a36e9aa86))
- fix: give agent-observer sub a replay-capable limit ([#1100](https://github.com/block/buzz/pull/1100)) ([`959fc6e9d`](https://github.com/block/buzz/commit/959fc6e9d0a68ff4de1e3b4dc3d731fb8f0b3b03))
- fix: make managed-agent spawn and teardown portable to Windows ([#1097](https://github.com/block/buzz/pull/1097)) ([`420131182`](https://github.com/block/buzz/commit/42013118215bbeb87a4fed30c5744e99bb0d8ae9))
- fix(desktop): constrain message timeline width with min-w-0 ([#1092](https://github.com/block/buzz/pull/1092)) ([`7de5517f1`](https://github.com/block/buzz/commit/7de5517f1d3d48e929e49fb8be20e04995428b50))
- feat(desktop): reminders notifications, snooze, overlay, and inbox view mode ([#1093](https://github.com/block/buzz/pull/1093)) ([`22b47db8e`](https://github.com/block/buzz/commit/22b47db8ecb80fb6deccad8323b38f7cbb1f5cf7))
- feat(prompt): add memory hygiene and hoist universal engineering discipline to base prompt ([#1085](https://github.com/block/buzz/pull/1085)) ([`010e8022b`](https://github.com/block/buzz/commit/010e8022b0b07b135db652d61afd7250c27e0564))
- fix(desktop): correct thread-unread badge flicker, stale clear, phantom count, mention gate, and nested count ([#1080](https://github.com/block/buzz/pull/1080)) ([`c8e1120fc`](https://github.com/block/buzz/commit/c8e1120fc2e78424526f49248b5d531a567a97b8))
- Fix mention chip alignment ([#1094](https://github.com/block/buzz/pull/1094)) ([`4f93d52e1`](https://github.com/block/buzz/commit/4f93d52e156a5683f114897d43f25f75704a292e))
- perf(desktop): virtualize unbounded lists and warm the emoji index ([#1089](https://github.com/block/buzz/pull/1089)) ([`a4fbebb39`](https://github.com/block/buzz/commit/a4fbebb397c0af4f19b3fc7af531ff067d8dead0))
- Adjust unread pill spacing ([#1091](https://github.com/block/buzz/pull/1091)) ([`3bf2ac004`](https://github.com/block/buzz/commit/3bf2ac00476440cc454c4d5b2ada87073c808500))
- Improve image lightbox controls ([#1084](https://github.com/block/buzz/pull/1084)) ([`bb9b6674e`](https://github.com/block/buzz/commit/bb9b6674e36a38d3a6395153930d2fb8ed502f45))
- Use Inter for app typography ([#899](https://github.com/block/buzz/pull/899)) ([`7b0500c93`](https://github.com/block/buzz/commit/7b0500c93e8923c0826a28af4fee3c88ed688280))
- fix(desktop): paint thread replies on open without scroll nudge ([#1090](https://github.com/block/buzz/pull/1090)) ([`422a90f5e`](https://github.com/block/buzz/commit/422a90f5e180d066128c51e45991278d39a707de))
- Polish sidebar update and relay cards ([#1009](https://github.com/block/buzz/pull/1009)) ([`d4cd919b8`](https://github.com/block/buzz/commit/d4cd919b8506469f0bfd68100f13cdf16d3e76bc))
- Normalize desktop icon sizing ([#1088](https://github.com/block/buzz/pull/1088)) ([`1a37ee395`](https://github.com/block/buzz/commit/1a37ee395119e5e54cb2798346bad4b5c9a5b29c))
- fix(desktop): normalize loopback host for HTTP writes and stop Reminders nav flicker ([#1086](https://github.com/block/buzz/pull/1086)) ([`771086fd6`](https://github.com/block/buzz/commit/771086fd69243b3bdccf213783190ad119486de9))
- feat(composer): link editing — inline popover, modal picker, buzz:// in-app nav ([#1045](https://github.com/block/buzz/pull/1045)) ([`770266d2a`](https://github.com/block/buzz/commit/770266d2a99925b92716b8991139f223d8ff5dda))
- chore(Pulse): gate Follow button behind Pulse feature flag ([#1019](https://github.com/block/buzz/pull/1019)) ([`48994e01b`](https://github.com/block/buzz/commit/48994e01bca2345d56fd0cf72458caa52686e2ec))
- fix(desktop): wrap long markdown autolinks ([#1081](https://github.com/block/buzz/pull/1081)) ([`b7e22f4b5`](https://github.com/block/buzz/commit/b7e22f4b5c13b995fe26587c2a52bbe0e775e0b9))
- feat(desktop): add NIP-ER reminder UI — create, view, and manage encrypted reminders ([#963](https://github.com/block/buzz/pull/963)) ([`ff824a365`](https://github.com/block/buzz/commit/ff824a365e95651edc08b7721e81e091734e8664))
- feat(relay): add NIP-ER push scheduler with cross-pod delivery ([#957](https://github.com/block/buzz/pull/957)) ([`26563968c`](https://github.com/block/buzz/commit/26563968cba232db7b16c0b696273819b5e2c571))
- feat(relay): implement NIP-ER event reminder support (kind:30300) ([#934](https://github.com/block/buzz/pull/934)) ([`79fcfd82b`](https://github.com/block/buzz/commit/79fcfd82bc3a883e1400449f87779853dc0e575c))
- feat(desktop): adding ui state into the history stack ([#967](https://github.com/block/buzz/pull/967)) ([`538f33341`](https://github.com/block/buzz/commit/538f33341fbdfa9794728dabd5ce995769dc151c))
- fix(desktop): restore timeline zoom via rem tokens + chat-as-base type scale ([#1052](https://github.com/block/buzz/pull/1052)) ([`c22c54e7a`](https://github.com/block/buzz/commit/c22c54e7ae4318f9648dc9441a152732cb29d6d5))
- fix(release): format changelog as linked markdown bullets ([#1075](https://github.com/block/buzz/pull/1075)) ([`d879fac4c`](https://github.com/block/buzz/commit/d879fac4c424550d5fa839861c53583cdf032af5))


## v0.3.24

- feat(desktop): refine thread-unread badge to two-token form ([#1069](https://github.com/block/buzz/pull/1069)) ([`de24b90aa`](https://github.com/block/buzz/commit/de24b90aa1076822c4de184c3bb8b5f6ebc682a2))
- fix(buzz): prevent reconnect storms from reaped ephemeral channels ([#1071](https://github.com/block/buzz/pull/1071)) ([`fd2553726`](https://github.com/block/buzz/commit/fd2553726c6a8277c472b44227d13546673117b2))
- fix(buzz-acp): trim oversized observer frames to fit instead of dropping ([#1072](https://github.com/block/buzz/pull/1072)) ([`5a651632d`](https://github.com/block/buzz/commit/5a651632d873d29d9867fc2a1445547dc519e95d))
- perf(ci): speed up PR CI wall clock and local dev builds ([#1028](https://github.com/block/buzz/pull/1028)) ([`07efae7b5`](https://github.com/block/buzz/commit/07efae7b5b8948ba12231436e7f2820b42d37496))
- chore(deps): update react monorepo ([#1048](https://github.com/block/buzz/pull/1048)) ([`56a5ac279`](https://github.com/block/buzz/commit/56a5ac27900661aa1674b93f81867d2930ccff93))
- Polish desktop visual details ([#1067](https://github.com/block/buzz/pull/1067)) ([`9f99e62d4`](https://github.com/block/buzz/commit/9f99e62d409fe7d5f9330de5641e9c6fcfc3b752))
- ci: use running postgres for pgschema desired-state planning ([#1070](https://github.com/block/buzz/pull/1070)) ([`e3736f08b`](https://github.com/block/buzz/commit/e3736f08b77b81d3752c3aceb1e25022e5dc759b))
- fix(desktop): anchor active-turn badge to skew-corrected agent start ([#1068](https://github.com/block/buzz/pull/1068)) ([`2d26db6d8`](https://github.com/block/buzz/commit/2d26db6d8a7381ff0dc9b50194fd28d42957de74))
- feat(desktop): add configurable transport reconnect hook ([#1059](https://github.com/block/buzz/pull/1059)) ([`ba776b995`](https://github.com/block/buzz/commit/ba776b9959324e10ffacf40126875783f0310ac0))
- Add automatic database migrations ([#988](https://github.com/block/buzz/pull/988)) ([`2300248d3`](https://github.com/block/buzz/commit/2300248d3b40359da7d92b6665041a70b5520047))
- Add composer spoiler formatting ([#1055](https://github.com/block/buzz/pull/1055)) ([`f8715612a`](https://github.com/block/buzz/commit/f8715612afb03f1c7938432141c9904c20e9b9ee))
- feat(desktop): in-channel and in-thread unread indicators ([#1008](https://github.com/block/buzz/pull/1008)) ([`2a2c1c800`](https://github.com/block/buzz/commit/2a2c1c8001a2eb6dbf35b3f13ec5468d1a7a1417))
- perf(timeline): gate heavy message render behind useDeferredValue ([#1022](https://github.com/block/buzz/pull/1022)) ([`cbc754cff`](https://github.com/block/buzz/commit/cbc754cffb0e14c25b380de6c2d5f8d62c14a183))
- Add animated profile avatars ([#1031](https://github.com/block/buzz/pull/1031)) ([`116486592`](https://github.com/block/buzz/commit/116486592d05f8ab079b2d26a8805d8fe7a055b5))
- Polish direct message and members modals ([#1054](https://github.com/block/buzz/pull/1054)) ([`89ae31d20`](https://github.com/block/buzz/commit/89ae31d2034eab36445be9b43d9edf51cd34576f))
- Polish huddles UI ([#1041](https://github.com/block/buzz/pull/1041)) ([`5234fc816`](https://github.com/block/buzz/commit/5234fc81675f46a0d616d336453c51f9b90f66cd))
- Fix video review comments in threads ([#1056](https://github.com/block/buzz/pull/1056)) ([`424ea7025`](https://github.com/block/buzz/commit/424ea70254f7703ec31688aa7096f145ec4a59a8))
- Polish message reaction tray ([#1002](https://github.com/block/buzz/pull/1002)) ([`81296d976`](https://github.com/block/buzz/commit/81296d9766af0f0ddecd9e14526cb9456952b489))
- Refine app loading skeletons ([#1001](https://github.com/block/buzz/pull/1001)) ([`c30d7274c`](https://github.com/block/buzz/commit/c30d7274c2febee6b274ea0160b9465cc93899b8))
- Polish channel modal forms ([#1000](https://github.com/block/buzz/pull/1000)) ([`ee34ca818`](https://github.com/block/buzz/commit/ee34ca818bc19a3f7f62f00debe4c7f2862b906d))
- Normalize desktop icon sizing ([#999](https://github.com/block/buzz/pull/999)) ([`19656fff4`](https://github.com/block/buzz/commit/19656fff46176e44ca7b4e8025f2fb7b34ee3589))
- Add shared skeleton loader primitives ([#998](https://github.com/block/buzz/pull/998)) ([`7a2e35521`](https://github.com/block/buzz/commit/7a2e35521ca103d1f2086fb195b7d91e588dc070))
- chore(scripts): update post-screenshots repo name to block/buzz ([#1042](https://github.com/block/buzz/pull/1042)) ([`9ee5aeebd`](https://github.com/block/buzz/commit/9ee5aeebd912940fc09ed9707ff716b63bc28409))
- docs: fix stale sprout repo references in RELEASING.md ([#1043](https://github.com/block/buzz/pull/1043)) ([`b2ad3074b`](https://github.com/block/buzz/commit/b2ad3074bacf8f29d179c80003228b4dd451efd0))


## v0.3.23

- fix(release): publish manifest from successful platforms ([#1039](https://github.com/block/buzz/pull/1039)) ([`9b410325a`](https://github.com/block/buzz/commit/9b410325a754305405dbaf80b63546df9af403ea))


## v0.3.22

- fix(release): publish rolling updater manifest for automated release tags


## v0.3.21

- fix(release): use signed NSIS installer for updates ([#1036](https://github.com/block/buzz/pull/1036)) ([`4d19a5901`](https://github.com/block/buzz/commit/4d19a5901d45aab7c615440e725e6d17d799e0da))
- handoff: pass full session history to summarizer ([#1033](https://github.com/block/buzz/pull/1033)) ([`fa1cade3b`](https://github.com/block/buzz/commit/fa1cade3ba76309bcbeac5e87e8932e2087622f2))
- feat(emoji): latest-set-wins union for custom emoji across desktop, mobile, and CLI ([#989](https://github.com/block/buzz/pull/989)) ([`6e4c8680c`](https://github.com/block/buzz/commit/6e4c8680c8c2346c8ff4612e79343d55b435d0d6))
- Fix relay NIP-11 software URL ([#1030](https://github.com/block/buzz/pull/1030)) ([`5f8ab33b3`](https://github.com/block/buzz/commit/5f8ab33b3b132decc35b80723fb8f197d37edda9))
- fix(desktop): make Windows release compile cleanly ([#1029](https://github.com/block/buzz/pull/1029)) ([`5c2f46e62`](https://github.com/block/buzz/commit/5c2f46e627e8a9c257e183bf98289c31757fc6b3))
- Add production Docker Compose bundle ([#985](https://github.com/block/buzz/pull/985)) ([`6caa359d7`](https://github.com/block/buzz/commit/6caa359d7052d494d1b2f55290d93d9e603e74dd))
- feat(profile): show active turn badges on agent profile panel and popover ([#1026](https://github.com/block/buzz/pull/1026)) ([`a32681fd4`](https://github.com/block/buzz/commit/a32681fd48e58355052d6980bc8f44d79cf4cd66))


## v0.3.20

- fix(release): resolve Windows sidecar path and Linux AppImage updater format ([#1024](https://github.com/block/buzz/pull/1024)) ([`c7dd4295b`](https://github.com/block/buzz/commit/c7dd4295b8feee4acd78bac86d1d92cf7e7463ad))


## v0.3.19

- fix(release): ignore prerelease tags in changelog generation ([#1021](https://github.com/block/buzz/pull/1021)) ([`faf00724f`](https://github.com/block/buzz/commit/faf00724fb4d187ad92a378b4069bf4fd1b4033f))
- fix: repair main build after cross-PR merge skew ([#1020](https://github.com/block/buzz/pull/1020)) ([`b8c0556e7`](https://github.com/block/buzz/commit/b8c0556e7cdda13c52ba1cf80e675e4fc40b6696))
- feat(agents): show per-turn duration and prune dead turns within ~25s of host crash ([#1017](https://github.com/block/buzz/pull/1017)) ([`87e45c65b`](https://github.com/block/buzz/commit/87e45c65bf9a0c841df2262f712a30c554888cac))
- fix(release): replace hermit with native tool setup on Windows job ([#1018](https://github.com/block/buzz/pull/1018)) ([`2fef8d664`](https://github.com/block/buzz/commit/2fef8d6643ad15268254d89ec84199c735142404))
- feat(acp): surface error-class outcomes to the activity feed only, never the channel ([#1010](https://github.com/block/buzz/pull/1010)) ([`6db90514b`](https://github.com/block/buzz/commit/6db90514b24d9c70073d52c9c8e8df332ccb82d5))
- fix(desktop): migrate Sprout workspace storage ([#1016](https://github.com/block/buzz/pull/1016)) ([`563f68434`](https://github.com/block/buzz/commit/563f684342cdf7b59642b124f9349b0a3ae3d284))
- feat(auth): force token refresh on rejected token (401/403), never the browser ([#1015](https://github.com/block/buzz/pull/1015)) ([`5a8cc79c6`](https://github.com/block/buzz/commit/5a8cc79c6a1d229b0ef7d33e1b98168d6d7841c0))
- fix(release): mark prerelease versions so they do not become latest ([#1013](https://github.com/block/buzz/pull/1013)) ([`59a7e5da8`](https://github.com/block/buzz/commit/59a7e5da8af296a51ff43fe8ff4b09e4bb9c48fa))
- feat(acp): implement systemPrompt with protocol version gating ([#981](https://github.com/block/buzz/pull/981)) ([`f08588245`](https://github.com/block/buzz/commit/f0858824548e97a73451ae6178a9ed14299cee42))
- fix(release): update repository name check from block/sprout to block/buzz ([#1012](https://github.com/block/buzz/pull/1012)) ([`d07c8216c`](https://github.com/block/buzz/commit/d07c8216cc1550a1a101f4e478d5deb3d60fa633))
- feat(release): all-OS desktop builds + universal auto-update manifest ([#1011](https://github.com/block/buzz/pull/1011)) ([`de641fce5`](https://github.com/block/buzz/commit/de641fce5247994958635244c29e2fd8fa51ccf0))
- Add relay disconnect UX: friendly errors, reconnect, cached identity ([#1004](https://github.com/block/buzz/pull/1004)) ([`8c9211ffc`](https://github.com/block/buzz/commit/8c9211ffc675000bdf55c4643528cf5d32a2d27a))
- feat(agents): add active turn indicators to Agents Menu ([#1005](https://github.com/block/buzz/pull/1005)) ([`7983bf675`](https://github.com/block/buzz/commit/7983bf6751f12562b2703dce73697e5cfb7f44da))
- ci: add fork guards to docker, release, and auto-tag workflows ([#1007](https://github.com/block/buzz/pull/1007)) ([`39d9aa826`](https://github.com/block/buzz/commit/39d9aa8260e0c8eb6fdc6277c8138b63688dad0c))
- docs(nip-rs): add optional thread read context scheme ([#1006](https://github.com/block/buzz/pull/1006)) ([`43d1ce353`](https://github.com/block/buzz/commit/43d1ce3532b769b23c20e0cb5228e007d574f0d3))
- fix(huddle): Pocket TTS quality overhaul — reference parity + cross-message pipelining ([#997](https://github.com/block/buzz/pull/997)) ([`12433077a`](https://github.com/block/buzz/commit/12433077add076dd89f5412a55ec3048b6b1b64d))
- Add manual ACP session rotation command ([#932](https://github.com/block/buzz/pull/932)) ([`00dc4915d`](https://github.com/block/buzz/commit/00dc4915d4b874c312a82fa707ee4d8f2437f61f))
- fix(desktop): heal stale persona_team_dir paths in release builds ([#1003](https://github.com/block/buzz/pull/1003)) ([`df8896f13`](https://github.com/block/buzz/commit/df8896f13ec5a937a443b968439016823f34e6b6))
- ci(docker): publish public ghcr.io/block/buzz image (native multi-arch) ([#986](https://github.com/block/buzz/pull/986)) ([`1fa63bada`](https://github.com/block/buzz/commit/1fa63badad283cff753ab403105344060fa74efa))
- fix(buzz-agent): cap tool-result text at 50 KiB with middle elision ([#952](https://github.com/block/buzz/pull/952)) ([`84f499cb6`](https://github.com/block/buzz/commit/84f499cb6ed65f8889ae7f0401451c02696b164e))
- feat(huddle): sentence-at-a-time voice-mode guidelines for lower TTS latency ([#996](https://github.com/block/buzz/pull/996)) ([`2846a96ed`](https://github.com/block/buzz/commit/2846a96ed2ed64077da3b7ad866e40c1cf26e952))
- Shard desktop Playwright CI jobs ([#992](https://github.com/block/buzz/pull/992)) ([`a1c28f487`](https://github.com/block/buzz/commit/a1c28f487d2af01d620619d940cf377d21c1a81a))


## v0.3.18

- Video Player Improvements ([#993](https://github.com/block/buzz/pull/993)) ([`05fc69b84`](https://github.com/block/buzz/commit/05fc69b84082d7e7e9360cab7ae9c9c0524511f9))
- Improve first-run welcome setup ([#970](https://github.com/block/buzz/pull/970)) ([`d9ce0943e`](https://github.com/block/buzz/commit/d9ce0943edb286404a3c99b501d53e7b35095955))
- fix(release): use legacy updater key secret ([#991](https://github.com/block/buzz/pull/991)) ([`50986406f`](https://github.com/block/buzz/commit/50986406ffa1f4e3dcff3e5373a3e0838d11fb2b))
- Replace built-in personas with Fizz ([#987](https://github.com/block/buzz/pull/987)) ([`ea5a0a9b4`](https://github.com/block/buzz/commit/ea5a0a9b40592d6192fb0ddbb99bf257009de5a8))
- docs(buzz-acp): rewrite Communication Patterns for mention accuracy and threading clarity ([#982](https://github.com/block/buzz/pull/982)) ([`654176541`](https://github.com/block/buzz/commit/6541765416b9da25dcd8c5c9da692b33d12782c7))
- chore(justfile): build git-credential-nostr in dev and staging recipes ([#980](https://github.com/block/buzz/pull/980)) ([`a101fd6ad`](https://github.com/block/buzz/commit/a101fd6ad38d4c2d8c668c2d5d79d2a35ab71176))
- Fix Buzz command migration for saved agents ([#979](https://github.com/block/buzz/pull/979)) ([`824c55114`](https://github.com/block/buzz/commit/824c55114ef388990f22df901caf9e37b1102b04))
- fix(desktop): resolve effective model and prompt from persona in display path ([#972](https://github.com/block/buzz/pull/972)) ([`63738139b`](https://github.com/block/buzz/commit/63738139bac3ce860fd6a0a50dc5883a374d6245))
- docs: clean up remaining Buzz references ([#977](https://github.com/block/buzz/pull/977)) ([`1bb8b8d54`](https://github.com/block/buzz/commit/1bb8b8d547a020622948225b32a8f9bbf3b0de2e))


## v0.3.17

- docs: finish Buzz rename cleanup ([#974](https://github.com/block/buzz/pull/974)) ([`79bcee55c`](https://github.com/block/buzz/commit/79bcee55cb717b117f267145a6b82d477f5aa428))
- fix(desktop): let channel members bypass mention agent gate ([#965](https://github.com/block/buzz/pull/965)) ([`6f3733d43`](https://github.com/block/buzz/commit/6f3733d43b4296b228f06874d10d22a7bb9a3c73))
- Rename desktop app to Buzz ([#960](https://github.com/block/buzz/pull/960)) ([`8f580f308`](https://github.com/block/buzz/commit/8f580f308cc3d92b07a65ca08cb117e8f7656a7d))
- feat(desktop): open profile panel from MembersSidebar rows ([#962](https://github.com/block/buzz/pull/962)) ([`dcb2639b3`](https://github.com/block/buzz/commit/dcb2639b355c988ca6c5529640cfd55c957edd05))
- feat(desktop): per-event notification sounds and alert controls ([#968](https://github.com/block/buzz/pull/968)) ([`4e4dc723e`](https://github.com/block/buzz/commit/4e4dc723e4cc1969efe9a8cc8d6e0e95a8e2d695))
- fix(desktop): make header chrome zoom-correct and tidy split-pane ([#941](https://github.com/block/buzz/pull/941)) ([`1ca16c898`](https://github.com/block/buzz/commit/1ca16c898c7ccd5c9a4f78768e697b411367c975))
- fix(desktop): rename SPROUT_ env vars to BUZZ_ for child agent processes ([#971](https://github.com/block/buzz/pull/971)) ([`8c8312932`](https://github.com/block/buzz/commit/8c8312932af9dced17fe1225d899885ec603b08b))
- fix(justfile): complete buzz rename in dev and staging recipes ([#966](https://github.com/block/buzz/pull/966)) ([`31b0665cf`](https://github.com/block/buzz/commit/31b0665cff7cb4735397696bd7ce03a17404d54a))
- refactor: rename sprout backend to buzz ([#958](https://github.com/block/buzz/pull/958)) ([`d99ad131f`](https://github.com/block/buzz/commit/d99ad131f176040549b041e297b813ef9705663e))
- fix(desktop): reap orphaned agent processes across instances ([#954](https://github.com/block/buzz/pull/954)) ([`53e3f0948`](https://github.com/block/buzz/commit/53e3f09485838fbff242c883ff412dd39d03ec27))
- Rename web app to Buzz ([#959](https://github.com/block/buzz/pull/959)) ([`c5a54dcc3`](https://github.com/block/buzz/commit/c5a54dcc3909a319e1d3604de4415142a3903619))
- fix(desktop): allow restarting saved relay-mesh agents from the UI ([#956](https://github.com/block/buzz/pull/956)) ([`510009c11`](https://github.com/block/buzz/commit/510009c11db58176c4af907414ecc994e36c6c0b))
- feat(acp): agent timeout resilience — idle margin, tool-call reset, death notices, keepalive ([#935](https://github.com/block/buzz/pull/935)) ([`60c8c5036`](https://github.com/block/buzz/commit/60c8c5036a20d10b8c32c95dd40bbb4d6098b59e))
- Rename mobile app to Buzz ([#955](https://github.com/block/buzz/pull/955)) ([`c63e018b0`](https://github.com/block/buzz/commit/c63e018b05b9e387b413f00417c2df6c5979fff5))
- fix(desktop): repair team-persona mismatch and deduplicate legacy imports ([#949](https://github.com/block/buzz/pull/949)) ([`929cc8861`](https://github.com/block/buzz/commit/929cc8861ee5629737fd9f7dafe4501ee071fa1b))
- fix(desktop): populate last_message_at in channel browser ([#951](https://github.com/block/buzz/pull/951)) ([`b792aa470`](https://github.com/block/buzz/commit/b792aa4704a2aeb669008c9c48736fc3acef35d9))
- Kit/circular avatars ([#927](https://github.com/block/buzz/pull/927)) ([`e5f0c3264`](https://github.com/block/buzz/commit/e5f0c32648be83908d3f4e06c8f746581b365527))
- fix(relay): accept mesh signaling kinds (24620/24621) via POST /events ([#946](https://github.com/block/buzz/pull/946)) ([`dbe973dac`](https://github.com/block/buzz/commit/dbe973dacd0165ad40689f28601660bb307c67ea))
- feat(sprout-dev-mcp): add read_file tool and replace_all to str_replace ([#928](https://github.com/block/buzz/pull/928)) ([`f49cdcdd3`](https://github.com/block/buzz/commit/f49cdcdd300dca846a7cd7023a29e5bf54fc96a1))


## v0.3.16

- fix(desktop): land live presence updates for not-yet-cached pubkeys ([#947](https://github.com/block/buzz/pull/947)) ([`34c8bdab1`](https://github.com/block/buzz/commit/34c8bdab1a8bde4fa6d24afcb081a60131e10ff3))
- fix(release): make `just release` idempotent for re-runs ([#948](https://github.com/block/buzz/pull/948)) ([`5c0af0bc9`](https://github.com/block/buzz/commit/5c0af0bc93d713beb1f77f6c9bc3dad65c0ff15b))
- Improve mentions for agents + people ([#942](https://github.com/block/buzz/pull/942)) ([`e9cd1c392`](https://github.com/block/buzz/commit/e9cd1c39264233317c1639d7389c1a843db7ef93))
- feat: agent memory viewer (read-only) in profile panel ([#917](https://github.com/block/buzz/pull/917)) ([`384eb6cba`](https://github.com/block/buzz/commit/384eb6cba4d589627f6ddf92d288c00dd988361f))
- Fix channel visibility controls ([#940](https://github.com/block/buzz/pull/940)) ([`2dc466fe0`](https://github.com/block/buzz/commit/2dc466fe0b45ee85e772af8ec5e47eeeb2051ce4))
- fix(delete): make agent-deleted messages disappear from desktop UI immediately ([#918](https://github.com/block/buzz/pull/918)) ([`3e56331e9`](https://github.com/block/buzz/commit/3e56331e9c2866d736d3f92fec03ef065d19007e))
- Fix emoji message rendering ([#938](https://github.com/block/buzz/pull/938)) ([`e08937cdd`](https://github.com/block/buzz/commit/e08937cddeff5702d8dfa3d7bfa187d2fbfcf305))
- refactor(desktop): consolidate packs into teams ([#852](https://github.com/block/buzz/pull/852)) ([`ba2fdbf69`](https://github.com/block/buzz/commit/ba2fdbf6978c8904b26e6f62a2d88782740d890c))
- Update README.md wording ([`384a34ec3`](https://github.com/block/buzz/commit/384a34ec31df67b10024e773c349fbd17030c51a))
- Fix post-compact handoff context for OpenAI providers ([#931](https://github.com/block/buzz/pull/931)) ([`fe14daa5d`](https://github.com/block/buzz/commit/fe14daa5d6a6dc681d3838b4f61a65d9b694be19))


## v0.3.15

- fix: persona is source of truth at spawn + thread-depth conventions ([#930](https://github.com/block/buzz/pull/930)) ([`877048d68`](https://github.com/block/buzz/commit/877048d68fb064947c8845d0051237064937660b))
- fix: skip avatar reconciliation for legacy agent records ([#933](https://github.com/block/buzz/pull/933)) ([`73cd8d082`](https://github.com/block/buzz/commit/73cd8d082d474a5cd16bdd4b404010027ed3ad17))
- feat(desktop): add nest commit identity guidance with human sign-off ([#929](https://github.com/block/buzz/pull/929)) ([`165b9f7a5`](https://github.com/block/buzz/commit/165b9f7a5f5ba3d5a48ff19d04c25b455058e537))
- feat: provider/model selection for personas and runtime-aware env injection ([#794](https://github.com/block/buzz/pull/794)) ([`9a98e60fc`](https://github.com/block/buzz/commit/9a98e60fc29edacfb27134c19676d114c2ce20a2))
- fix: reconcile agent profile on startup when relay publish was missed ([#921](https://github.com/block/buzz/pull/921)) ([`762a45969`](https://github.com/block/buzz/commit/762a45969a6bec0e6fe3efe6b86c2aa1f47d7eeb))
- Revamp first-run onboarding ([#924](https://github.com/block/buzz/pull/924)) ([`5d927aba0`](https://github.com/block/buzz/commit/5d927aba0c45a4a139b559be89bb1bb1595e7fdb))
- Update setup loading screen ([#926](https://github.com/block/buzz/pull/926)) ([`f36265019`](https://github.com/block/buzz/commit/f3626501952139974e641fecae3dc17fcfade187))
- fix(dm): keep hidden DMs hidden across refetch via relay-signed visibility snapshot (NIP-DV) ([#857](https://github.com/block/buzz/pull/857)) ([`c38301bca`](https://github.com/block/buzz/commit/c38301bca5a3d8015f1e7340b4e732ef8c0e3744))
- Maximize desktop window on launch ([#925](https://github.com/block/buzz/pull/925)) ([`4dfae61f2`](https://github.com/block/buzz/commit/4dfae61f24271fa2e0616e3d896b5f72f3a3ca0f))
- feat: preview features (experiments settings UI) ([#888](https://github.com/block/buzz/pull/888)) ([`ae430d4dd`](https://github.com/block/buzz/commit/ae430d4dd954fc8cf9e96ea6ac6940714cde9181))
- fix(updater): send no-cache header on update check to avoid stale manifest ([#922](https://github.com/block/buzz/pull/922)) ([`a357e220d`](https://github.com/block/buzz/commit/a357e220d82230277a123d809e3130dc359f0fbb))
- fix(desktop): refresh channel state after unarchive ([#923](https://github.com/block/buzz/pull/923)) ([`56230c144`](https://github.com/block/buzz/commit/56230c1442f9881278910f6ecbc5bb66178e9bd6))
- Add channel visibility & ephemeral TTL controls to manage sidebar ([#911](https://github.com/block/buzz/pull/911)) ([`7dd5b3453`](https://github.com/block/buzz/commit/7dd5b34536b82cb072b92a7e8000f543b4170d3a))
- ci(release): add Intel macOS (x86_64) DMG as a release target ([#748](https://github.com/block/buzz/pull/748)) ([`4b78fe3be`](https://github.com/block/buzz/commit/4b78fe3bea22c44568bc656db74e577c748d7627))
- mesh: Rust-owned coordinator — fix saved-agent reconnect flakiness + DRY the start path ([#879](https://github.com/block/buzz/pull/879)) ([`421593062`](https://github.com/block/buzz/commit/421593062f25ab19e9ce9fbbaa204d843231657f))


## v0.3.14

- fix(sdk): resolve multi-word display names and add NIP-27 nostr:npub mention extraction ([#905](https://github.com/block/buzz/pull/905)) ([`bfafdd46b`](https://github.com/block/buzz/commit/bfafdd46b29a04421efdd95ee0a157e2da86ee54))
- fix(desktop): re-enable mcp_command reconciliation and harden spawn site ([#909](https://github.com/block/buzz/pull/909)) ([`15f610dcd`](https://github.com/block/buzz/commit/15f610dcd5c1d1308beed167c316721b2deefc2d))
- Fix desktop DM and sidebar UI polish ([#908](https://github.com/block/buzz/pull/908)) ([`da80c7340`](https://github.com/block/buzz/commit/da80c7340f300cd349cfee15b8f61c9a2f576b92))
- Animate reaction counts ([#904](https://github.com/block/buzz/pull/904)) ([`dd08f988d`](https://github.com/block/buzz/commit/dd08f988dec85f42f2202bfad0589278a3d22a34))
- Mobile custom emoji + settings redesign ([#906](https://github.com/block/buzz/pull/906)) ([`10b6674bd`](https://github.com/block/buzz/commit/10b6674bd7907e6bf3d81cc2a3f128d896d233f2))
- Renew TTL when unarchiving ephemeral channels ([#902](https://github.com/block/buzz/pull/902)) ([`732e23dd5`](https://github.com/block/buzz/commit/732e23dd5c37becd0ae24f4afdca757ff0b85264))


## v0.3.13

- Collapse channel header actions ([#901](https://github.com/block/buzz/pull/901)) ([`ecca5e77e`](https://github.com/block/buzz/commit/ecca5e77e4ec973ddeab1d0f21981d97958515f9))
- sprout-agent: make Databricks defaults env-only ([#868](https://github.com/block/buzz/pull/868)) ([`4ec7f8125`](https://github.com/block/buzz/commit/4ec7f8125e845783f3329ab7f2f7193dd2d83938))
- Restyle settings sections ([#894](https://github.com/block/buzz/pull/894)) ([`b384354e2`](https://github.com/block/buzz/commit/b384354e2bd110a511ed8d9d045419939f4d1d2b))
- Add emoji reaction particles ([#890](https://github.com/block/buzz/pull/890)) ([`fdcbb696f`](https://github.com/block/buzz/commit/fdcbb696fe0797fe9777da7db3116df366cb54ea))
- Move settings into the app shell ([#893](https://github.com/block/buzz/pull/893)) ([`32039b9a2`](https://github.com/block/buzz/commit/32039b9a25b4f7feb625662391f09b55fe6090ca))
- Tune chat text sizing ([#891](https://github.com/block/buzz/pull/891)) ([`45f3dfe5b`](https://github.com/block/buzz/commit/45f3dfe5ba62bfda2d1279814e52f5954b489a28))
- Style channel header navigation ([#889](https://github.com/block/buzz/pull/889)) ([`29f6ccf9e`](https://github.com/block/buzz/commit/29f6ccf9e9c54acc05d106e0be33bb366ebf3f02))
- fix: rename missed known_acp_provider_exact → known_acp_runtime_exact ([#900](https://github.com/block/buzz/pull/900)) ([`2ebe55174`](https://github.com/block/buzz/commit/2ebe55174106164dc56834f223c549b7a073b0aa))
- chore(deps): update radix-ui-primitives monorepo ([#898](https://github.com/block/buzz/pull/898)) ([`97bdb79de`](https://github.com/block/buzz/commit/97bdb79ded545dc7c91b1dfd6a01e32f631b8df9))
- chore(deps): update actions/checkout digest to df4cb1c ([#897](https://github.com/block/buzz/pull/897)) ([`4a93100e1`](https://github.com/block/buzz/commit/4a93100e199a2c7c623d4b2db0e11b26684330c9))
- refactor: rename ACP "provider" to "runtime" across the codebase ([#783](https://github.com/block/buzz/pull/783)) ([`0a6067ca1`](https://github.com/block/buzz/commit/0a6067ca1bebd48b84e6fcd63db12d255a10071f))
- Unify avatar radius ([#892](https://github.com/block/buzz/pull/892)) ([`056b87d3d`](https://github.com/block/buzz/commit/056b87d3da4584cef3b9d8d8e1807731fb16ea64))


## v0.3.12

- Show hover cards for inline message emoji ([#885](https://github.com/block/buzz/pull/885)) ([`1b7b6978f`](https://github.com/block/buzz/commit/1b7b6978fc83dc2001ce0a04380c978bd57f7a7e))
- Fix monotonic read-state merges ([#884](https://github.com/block/buzz/pull/884)) ([`5268fac2d`](https://github.com/block/buzz/commit/5268fac2d840bb77f2c64d358ef91fa173f8295f))
- Refine sidebar behavior and borders ([#869](https://github.com/block/buzz/pull/869)) ([`0a4783c6f`](https://github.com/block/buzz/commit/0a4783c6f8afb02355bded483cecf2a2eb3abc68))
- fix(presence): clear on disconnect, fix heartbeat/TTL, drop broken REST path ([#877](https://github.com/block/buzz/pull/877)) ([`5d7c74896`](https://github.com/block/buzz/commit/5d7c74896989938def2d54637cad21bd1e18d0c1))
- fix(cli): publish ephemeral events over WebSocket via sprout-ws-client ([#876](https://github.com/block/buzz/pull/876)) ([`ef98ae942`](https://github.com/block/buzz/commit/ef98ae942a5768153c72fca8c38c0255375f46b5))
- docs(sprout-acp): add communication discipline rules to base prompt + deprecate --mention flag ([#883](https://github.com/block/buzz/pull/883)) ([`2f50011bd`](https://github.com/block/buzz/commit/2f50011bdd25406ff47140b70b020ccf3cfbf6be))
- Polish thread summaries and reactions ([#881](https://github.com/block/buzz/pull/881)) ([`5c2476a71`](https://github.com/block/buzz/commit/5c2476a71e45e6c94fcb6d586e6a3658feb6b262))
- feat(cli): add emoji export and import subcommands ([#882](https://github.com/block/buzz/pull/882)) ([`7129cd6f2`](https://github.com/block/buzz/commit/7129cd6f23e4191fc8e4f092da91324982b3edec))
- Polish message row hover states ([#880](https://github.com/block/buzz/pull/880)) ([`b84f8e6a0`](https://github.com/block/buzz/commit/b84f8e6a010b837608f0e7c755e82a38367c4a73))
- Improve emoji naming and custom emoji UX ([#878](https://github.com/block/buzz/pull/878)) ([`bc5300867`](https://github.com/block/buzz/commit/bc53008676fc9b67209a809167d8f3fb52c0c03a))
- docs: add ecosystem section to CONTRIBUTING.md, fix stale release info ([#873](https://github.com/block/buzz/pull/873)) ([`581c7e95a`](https://github.com/block/buzz/commit/581c7e95a9b0ee847d20cdccc803c1f773a89621))
- fix(relay): wire custom filter fields through HTTP bridge ([#864](https://github.com/block/buzz/pull/864)) ([`031152221`](https://github.com/block/buzz/commit/031152221bdbb0cad9f9046f2ea999912463427f))
- chore: deprecate sprout-mcp — fill CLI gaps, remove crate and all references ([#850](https://github.com/block/buzz/pull/850)) ([`f1c672fea`](https://github.com/block/buzz/commit/f1c672fea53de252055d96bb76685a5bef731b9b))
- Fix custom emoji status in profile popover ([#874](https://github.com/block/buzz/pull/874)) ([`5bdac0566`](https://github.com/block/buzz/commit/5bdac0566fb8e73be99cfc4540af12f68a90448f))
- fix(agent): gate handoff on provider token usage, not byte estimate ([#821](https://github.com/block/buzz/pull/821)) ([`b295f51c9`](https://github.com/block/buzz/commit/b295f51c904946e78071ac6e45297396c88ee87f))
- docs: add VISION_MESH.md — the compute-commons vision ([#867](https://github.com/block/buzz/pull/867)) ([`cdb7bc27e`](https://github.com/block/buzz/commit/cdb7bc27e1183e7616b477877f2e8273ff26db8c))
- fix(desktop): simplify profile popover header ([#853](https://github.com/block/buzz/pull/853)) ([`d4bb7f66e`](https://github.com/block/buzz/commit/d4bb7f66e0df89682ceff0765f57b5e27f1ef8b3))
- fix(desktop): remove thread comment hover outline ([#861](https://github.com/block/buzz/pull/861)) ([`ccff5464a`](https://github.com/block/buzz/commit/ccff5464a41fce777d7f00fab15fec459bbac322))
- feat(desktop): always show channel section search/add buttons ([#856](https://github.com/block/buzz/pull/856)) ([`ad7ab482e`](https://github.com/block/buzz/commit/ad7ab482eb16bed63209d473110a2006a2e6caeb))


## v0.3.11

- fix(mobile+desktop): cross-device read state sync + diagnostic logging ([#843](https://github.com/block/buzz/pull/843)) ([`269b35e8d`](https://github.com/block/buzz/commit/269b35e8de7b2cb6b6908b1bddee9460774b2802))
- feat(mobile): star channels (Slack-style favorites) ([#863](https://github.com/block/buzz/pull/863)) ([`3ddfe5fcc`](https://github.com/block/buzz/commit/3ddfe5fcc25992bd0733d5013589a3521c9989f0))
- feat: desktop-screenshot skill to stop agents uploading relay media to PRs ([#862](https://github.com/block/buzz/pull/862)) ([`36d7dbd7c`](https://github.com/block/buzz/commit/36d7dbd7cae45311252a6c10bdf915c0a3dd3e44))
- feat(desktop): star channels (Slack-style favorites) ([#860](https://github.com/block/buzz/pull/860)) ([`c10b4f8f5`](https://github.com/block/buzz/commit/c10b4f8f5c646a5d89dfc29bb14ca1c0aa11faea))
- fix(desktop): handle symlinked persona pack directories ([#859](https://github.com/block/buzz/pull/859)) ([`f748f7126`](https://github.com/block/buzz/commit/f748f71268eb4f61e0e1dde5d216068b76e69804))
- feat: channel muting for desktop and mobile ([#838](https://github.com/block/buzz/pull/838)) ([`1fe7bf287`](https://github.com/block/buzz/commit/1fe7bf2872561d2e2f36b0db4202611fb665f676))
- feat(acp): default SPROUT_ACP_MEMORY to on ([#854](https://github.com/block/buzz/pull/854)) ([`4ead7de46`](https://github.com/block/buzz/commit/4ead7de46301efa4743df5b4c5f06374de537c6c))
- fix(desktop): eliminate image-hover layout jump in messages ([#813](https://github.com/block/buzz/pull/813)) ([`759e5cd92`](https://github.com/block/buzz/commit/759e5cd923544c307b5b81992ae6010df437bdd2))


## v0.3.10

- fix(desktop): harden relay mesh connect p-tag ([#834](https://github.com/block/buzz/pull/834)) ([`34ac3ba1d`](https://github.com/block/buzz/commit/34ac3ba1d1825e1dfbe5d81dd67e9b7375b29785))
- fix(desktop): scroll activity panel to bottom on open ([#848](https://github.com/block/buzz/pull/848)) ([`b3aefae15`](https://github.com/block/buzz/commit/b3aefae152ebc68a7f8d8720396f3e6ec06c947b))
- Polish desktop profile menu interactions ([#836](https://github.com/block/buzz/pull/836)) ([`a13691b62`](https://github.com/block/buzz/commit/a13691b6207fedb1b663ec1458bdea76bb6ec64c))
- fix(desktop): outline thread hover targets ([#845](https://github.com/block/buzz/pull/845)) ([`0d9b8148f`](https://github.com/block/buzz/commit/0d9b8148f86e39651871e438cad4b17560d5ffef))
- fix(desktop): keep message actions hover-only ([#844](https://github.com/block/buzz/pull/844)) ([`b3be9ecba`](https://github.com/block/buzz/commit/b3be9ecba70f17c12096e52489a6f7de3e00cd56))
- fix(desktop): let inbox composer fill available width ([#841](https://github.com/block/buzz/pull/841)) ([`db46c4254`](https://github.com/block/buzz/commit/db46c4254636561fcbec6f63478997102caab770))
- fix: use immutable commit-SHA URLs in screenshot PR comments ([#842](https://github.com/block/buzz/pull/842)) ([`2a0572c0d`](https://github.com/block/buzz/commit/2a0572c0d8e4d85afd58e161ade2f927bf19d76d))
- feat(mobile+desktop): two-tier Slack-style app icon badge ([#802](https://github.com/block/buzz/pull/802)) ([`3b78dc569`](https://github.com/block/buzz/commit/3b78dc5690b915822fdbe82c20b1bd565ab26108))
- chore: simplify file-size check to a flat 1000-line limit ([#839](https://github.com/block/buzz/pull/839)) ([`0c225f4d7`](https://github.com/block/buzz/commit/0c225f4d7d32d79ba38fb154fa878564d6f851de))
- fix(desktop): robust emoji picker — unify picker + fix custom emoji in editing, status, reactions ([#837](https://github.com/block/buzz/pull/837)) ([`d8b602a35`](https://github.com/block/buzz/commit/d8b602a359508c816bb7bee468a3e5eb389e3f81))
- feat(desktop): reusable screenshot workflow for agents ([#826](https://github.com/block/buzz/pull/826)) ([`06bc67fd3`](https://github.com/block/buzz/commit/06bc67fd342b658bd3ade26d23ac9d63bfb0392a))
- desktop(mesh-llm): let a serving node route a different model ([#833](https://github.com/block/buzz/pull/833)) ([`9f0c22a43`](https://github.com/block/buzz/commit/9f0c22a43a6b13030e0e0f75e25fca9983710a18))


## v0.3.9

- fix: native arbitrary-file download + image context-menu flash ([#830](https://github.com/block/buzz/pull/830)) ([`82ae85f79`](https://github.com/block/buzz/commit/82ae85f79a9e687b51a599f44cf8e8b47b756b80))
- fix(desktop): custom emoji reaction rendering + picker autofocus ([#831](https://github.com/block/buzz/pull/831)) ([`7797ae77f`](https://github.com/block/buzz/commit/7797ae77f6412972a95a9f3ffd90555fbd2d1051))
- Mesh-LLM v1: relay-gated direct-iroh inference between users (WAN) ([#822](https://github.com/block/buzz/pull/822)) ([`33cfc8529`](https://github.com/block/buzz/commit/33cfc8529325b99ebdf4afc1c4761a9ed87dad86))


## v0.3.8



## v0.3.7

- feat: custom emoji — user-owned NIP-30 sets with a client-side union ([#816](https://github.com/block/buzz/pull/816)) ([`234942130`](https://github.com/block/buzz/commit/2349421304d84b0b2537bebf2cf7923a1ce30e27))
- Install sprout-cli skill at repo root + fix desktop clippy ([#818](https://github.com/block/buzz/pull/818)) ([`7a12e5051`](https://github.com/block/buzz/commit/7a12e50518ec51db23c83301b99c66e9bbfdb95f))
- fix(desktop): use public re-export path for ensure_client_node_for_model ([#824](https://github.com/block/buzz/pull/824)) ([`2ea3fd88d`](https://github.com/block/buzz/commit/2ea3fd88d23966985e06af494a556fbdd4efc300))
- refactor(desktop): feature-gate mesh-llm-sdk behind optional Cargo feature ([#823](https://github.com/block/buzz/pull/823)) ([`fb514c891`](https://github.com/block/buzz/commit/fb514c8918d06b1b76e16c78fa6397c2871d80b4))
- fix(desktop): align workflow read/save commands to the frontend contract ([#820](https://github.com/block/buzz/pull/820)) ([`b72eee365`](https://github.com/block/buzz/commit/b72eee365f02465334869c5e48fd6136a829d39c))
- fix(desktop): disable mesh-llm auto-build to prevent git config corruption ([#819](https://github.com/block/buzz/pull/819)) ([`5b572d6f5`](https://github.com/block/buzz/commit/5b572d6f5e94ff4d8199a4da40342f301a2896e6))
- fix(desktop): clear clippy lints in agents/mesh_llm commands ([#817](https://github.com/block/buzz/pull/817)) ([`192388a3c`](https://github.com/block/buzz/commit/192388a3cbf7df254e0117be8459574282f52deb))
- fix(desktop): let channel members add members and bots without admin ([#815](https://github.com/block/buzz/pull/815)) ([`41a3fc158`](https://github.com/block/buzz/commit/41a3fc1589bed81d0ae856036dcf4676b53e1969))
- Desktop #806 follow-ups: panel/inbox fixes + top-bar backdrop ([#814](https://github.com/block/buzz/pull/814)) ([`a25ca5d1b`](https://github.com/block/buzz/commit/a25ca5d1bfbc0bfb7c8a2a2c0ca9bb093423d09e))
- Fix desktop right-side panel chrome overlap ([#806](https://github.com/block/buzz/pull/806)) ([`5bed17a11`](https://github.com/block/buzz/commit/5bed17a1173457d61760cfb5a7ba6840e83ec0cd))
- Sprout × mesh-llm: in-process mesh node (serve/consume) + relay admission ([#798](https://github.com/block/buzz/pull/798)) ([`ede8ddb42`](https://github.com/block/buzz/commit/ede8ddb425ccc09130949752e374a8bcaf842ad1))
- fix(desktop): resolve flaky integration tests via project-level assertion timeout ([#812](https://github.com/block/buzz/pull/812)) ([`6481428e2`](https://github.com/block/buzz/commit/6481428e2b9a505c771277478b50b01a01242c5e))


## v0.3.6

- feat(mobile): add channel sections with relay sync ([#800](https://github.com/block/buzz/pull/800)) ([`5cbedb180`](https://github.com/block/buzz/commit/5cbedb180af56274a518c5233e4de918313bb8d1))
- feat(desktop): sync channel sections across devices via Nostr ([#792](https://github.com/block/buzz/pull/792)) ([`753d0fe26`](https://github.com/block/buzz/commit/753d0fe264d4d997f8291f801547242420ba7d97))
- feat(media): support arbitrary file types with download cards ([#810](https://github.com/block/buzz/pull/810)) ([`2b052eb46`](https://github.com/block/buzz/commit/2b052eb465f305fc02a535e96012665da34d4d6b))
- feat(desktop): add user-defined channel sections to sidebar ([#789](https://github.com/block/buzz/pull/789)) ([`247ac5239`](https://github.com/block/buzz/commit/247ac5239151af6537ae7a16c43c7e99bbe4463e))
- feat(desktop): keyboard shortcuts — ⌘⇧N new channel + ↑-to-edit last message ([#809](https://github.com/block/buzz/pull/809)) ([`d810608d8`](https://github.com/block/buzz/commit/d810608d85926146e5d37890635919c022e761a0))
- fix(desktop): scope agent sweep to the owning app instance ([#808](https://github.com/block/buzz/pull/808)) ([`39911e428`](https://github.com/block/buzz/commit/39911e42859df41d8ea88195c82e010eb3d98283))
- fix(desktop): route notification clicks to thread context ([#790](https://github.com/block/buzz/pull/790)) ([`f2c266bac`](https://github.com/block/buzz/commit/f2c266bac2349f8d5e34c22d42ef0e55c227f36d))
- chore(deps): update all non-major dependencies ([#804](https://github.com/block/buzz/pull/804)) ([`33e37de6e`](https://github.com/block/buzz/commit/33e37de6e37cbb73e1ac2ef2499a280f6fcce3a8))
- fix(deps): re-pin isomorphic-git patch to 1.38.3 ([#807](https://github.com/block/buzz/pull/807)) ([`5670ffc6a`](https://github.com/block/buzz/commit/5670ffc6a6ec7d6ae6b7b2be4006e23580a10801))
- chore(deps): update dependency @tanstack/react-query to v5.100.14 ([#805](https://github.com/block/buzz/pull/805)) ([`033d92f10`](https://github.com/block/buzz/commit/033d92f103ea74fd577f0c2cacc845b1399a5c23))
- Fix desktop glass chrome and inbox previews ([#793](https://github.com/block/buzz/pull/793)) ([`bc23620fc`](https://github.com/block/buzz/commit/bc23620fcfae05813b5767d6b0eb830ab108b2b4))
- refactor(just): slim down mobile-dev to just run Flutter ([#801](https://github.com/block/buzz/pull/801)) ([`9b9cf4612`](https://github.com/block/buzz/commit/9b9cf461278f0d554523f76cbf80f49e80980bdf))
- refactor: consolidate infra management into justfile + add mobile-dev ([#797](https://github.com/block/buzz/pull/797)) ([`2a0385152`](https://github.com/block/buzz/commit/2a03851527f2fad199581e49ea5168cc7304b090))


## v0.3.5

- feat(mobile): Pulse polish — flat feed, compose page, shared filter chips, like + accent fixes ([#796](https://github.com/block/buzz/pull/796)) ([`b82042090`](https://github.com/block/buzz/commit/b820420909d225b18c45f1bd330a00a2edbee09b))
- feat(desktop): add standalone Playwright screenshot helper ([#795](https://github.com/block/buzz/pull/795)) ([`10f37e4ca`](https://github.com/block/buzz/commit/10f37e4cab244c6da319bbad803d65cf3981b794))
- feat(sprout-agent): load AGENTS.md and SKILL.md into system prompt ([#762](https://github.com/block/buzz/pull/762)) ([`f34a21d32`](https://github.com/block/buzz/commit/f34a21d32ff31e59a1ff2c11ce4857a483a0a806))
- feat: add code block support to message composer ([#788](https://github.com/block/buzz/pull/788)) ([`85861f33f`](https://github.com/block/buzz/commit/85861f33fe006226b59ab8273d7c357af09a781d))
- fix(desktop): reap orphaned agent processes on shutdown and restart ([#787](https://github.com/block/buzz/pull/787)) ([`7beb0f8e6`](https://github.com/block/buzz/commit/7beb0f8e6855c2cbef3c9e6310cff5a7368bdd33))


## v0.3.4

- Update desktop navigation chrome and search ([#779](https://github.com/block/buzz/pull/779)) ([`d77b111b1`](https://github.com/block/buzz/commit/d77b111b1531a7f2992ab7b60edb975549707b4f))
- feat(desktop): reload webview on Cmd/Ctrl+R ([#785](https://github.com/block/buzz/pull/785)) ([`5ee2cd051`](https://github.com/block/buzz/commit/5ee2cd05173d5bb5a8711753394ed23ab82f0f4e))
- fix(desktop): sync persona pack directory across worktree instances ([#782](https://github.com/block/buzz/pull/782)) ([`fa7febe40`](https://github.com/block/buzz/commit/fa7febe40f57d2bbf019d42e0079618625783ce7))


## v0.3.3

- fix(release): sync release tags during preflight ([#780](https://github.com/block/buzz/pull/780)) ([`c761a76ff`](https://github.com/block/buzz/commit/c761a76ff2d5789c57159c46bcf08fd02e16d82a))
- feat(desktop): thread-aware notifications with mutable follow/mute controls ([#761](https://github.com/block/buzz/pull/761)) ([`3f3ec6479`](https://github.com/block/buzz/commit/3f3ec64791b7cd40ddaa8d8332e00e6f11c09a8d))
- fix(desktop): improve model picker message and runtime dropdown clarity ([#778](https://github.com/block/buzz/pull/778)) ([`03e678cfb`](https://github.com/block/buzz/commit/03e678cfbcaa04d5bcfe11ff151c47e5007ba247))
- desktop: float unread indicator + fix sidebar scroll jump ([#777](https://github.com/block/buzz/pull/777)) ([`9db8f6ccb`](https://github.com/block/buzz/commit/9db8f6ccb887c67b90fcd6c9972aaa423b34c696))
- chore(hooks): standardize check/fix convention with auto-fix pre-commit ([#776](https://github.com/block/buzz/pull/776)) ([`3fbee555f`](https://github.com/block/buzz/commit/3fbee555f067ef80a75bfc52388a40bc336bf6e8))
- web: clickable repo tree + per-file blob viewer ([#773](https://github.com/block/buzz/pull/773)) ([`0f89ad169`](https://github.com/block/buzz/commit/0f89ad16925038db5d22e8b15ab7f69898c77b56))
- fix(agent): keep parallel tool-result messages contiguous on OpenAI Chat (Databricks image fix) ([#770](https://github.com/block/buzz/pull/770)) ([`61297ac80`](https://github.com/block/buzz/commit/61297ac80a30a13e63d4f01178aec1d33573b193))
- fix(release): fetch tags so changelog tracks versions correctly ([#775](https://github.com/block/buzz/pull/775)) ([`5f2423c23`](https://github.com/block/buzz/commit/5f2423c231e393dbc870c75be31f9ea61d9d6328))


## v0.3.2

- feat(mobile): add Pulse social feed tab ([#772](https://github.com/block/buzz/pull/772)) ([`1218572fa`](https://github.com/block/buzz/commit/1218572fa0380b5517a3c6a2aff34bdc51fa90e6))
- feat(sidebar): add More unread floating buttons ([#771](https://github.com/block/buzz/pull/771)) ([`fec6a683e`](https://github.com/block/buzz/commit/fec6a683ea88c25c76f29d5eb4d39709765a791b))
- chore: improve markdown spacing ([#766](https://github.com/block/buzz/pull/766)) ([`8eedec740`](https://github.com/block/buzz/commit/8eedec74030ce9cc32366def02c933267f853e80))
- fix: prevent inline links rendering in 2-column grid layout ([#767](https://github.com/block/buzz/pull/767)) ([`835a44aad`](https://github.com/block/buzz/commit/835a44aad03e782eeb76d716062767617b47e51b))

## v0.3.1

- [codex] Default release command to patch bump ([#768](https://github.com/block/buzz/pull/768)) ([`4222a758c`](https://github.com/block/buzz/commit/4222a758c6491fbbae113e862eb732c769a3dcf8))
- Polish desktop Pulse and Home views ([#764](https://github.com/block/buzz/pull/764)) ([`30654e95f`](https://github.com/block/buzz/commit/30654e95f6550854a16ce46bfbbd48dbcaa1b645))

## v0.3.0

- Initial release on the automated pipeline. Unifies OSS and internal version numbering above both v0.0.21 (OSS) and v0.2.38 (internal).
