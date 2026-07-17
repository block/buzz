# Changelog

## v0.4.13

- fix(desktop): compare relay media origins case-insensitively so uploads render when the saved community URL has uppercase characters
- fix(desktop): invalidate stale media lookups so an in-flight relay-origin fetch cannot repopulate caches after a community reset

## v0.4.12

- fix(desktop): proxy authenticated relay media after cold-start community initialization
- fix(desktop): keep shared-compute consumers admitted (WIP: multi-workspace publishing) ([#2000](https://github.com/block/buzz/pull/2000)) ([`5d77fa57`](https://github.com/block/buzz/commit/5d77fa57))
- feat(desktop): reskin provider, config, community, profile, and team onboarding pages ([#2003](https://github.com/block/buzz/pull/2003)) ([`f054df73`](https://github.com/block/buzz/commit/f054df73))


## v0.4.11

- Fix policy receipts for cold-launch invite links ([#2005](https://github.com/block/buzz/pull/2005)) ([`1d8d006a`](https://github.com/block/buzz/commit/1d8d006aaf078ffd5b92eae06334b7bb908b680e))
- Preserve persistent agent audience order ([#1989](https://github.com/block/buzz/pull/1989)) ([`fb8e6827`](https://github.com/block/buzz/commit/fb8e6827c758c73c9c6e16965dcdcecd00a5827c))
- feat(desktop): create channels in sections ([#1996](https://github.com/block/buzz/pull/1996)) ([`9b649740`](https://github.com/block/buzz/commit/9b6497402f343e380f6c79b97a01c2d52bd12f17))
- feat(desktop): graduate community rail ([#1995](https://github.com/block/buzz/pull/1995)) ([`f06b59ff`](https://github.com/block/buzz/commit/f06b59ff13bef0c031c8e4c3bdf979e120ea76d5))


## v0.4.10

- feat(desktop): onboarding step dots, identity/key page restyle, overflow scroll ([#1993](https://github.com/block/buzz/pull/1993)) ([`f2522995c`](https://github.com/block/buzz/commit/f2522995cbea67c96b79d5de6c19c665e465027d))
- chore(desktop): add AppShell.tsx file-size override to unblock main CI ([#1992](https://github.com/block/buzz/pull/1992)) ([`db57bc8a2`](https://github.com/block/buzz/commit/db57bc8a2cdac846d6e0f47100cb76bf07a4f326))
- feat: add invite QR and mobile direct join ([#1957](https://github.com/block/buzz/pull/1957)) ([`648cbf361`](https://github.com/block/buzz/commit/648cbf36109d97be6bd8530e77073d1c7e6008a0))
- feat: open add community from deep links ([#1970](https://github.com/block/buzz/pull/1970)) ([`adb48311e`](https://github.com/block/buzz/commit/adb48311e6e8940c83e9b75ab896807e09263960))
- fix(desktop): enforce observer archive policy reconciliation on internal builds ([#1923](https://github.com/block/buzz/pull/1923)) ([`896974554`](https://github.com/block/buzz/commit/896974554937126ec1d9e85f681da84fed9644b2))
- feat(desktop): streamline nostr identity pairing ([#1974](https://github.com/block/buzz/pull/1974)) ([`1eae8575e`](https://github.com/block/buzz/commit/1eae8575ed5a475729759315686a7d336f02c663))
- feat(desktop): ensure starter channels during onboarding ([#1937](https://github.com/block/buzz/pull/1937)) ([`74f3f2975`](https://github.com/block/buzz/commit/74f3f29758b0b76377a1823e22d65e65ce19ba41))
- test(desktop): make omission-preservation test discriminate on instructions ([#1988](https://github.com/block/buzz/pull/1988)) ([`2b015cb5d`](https://github.com/block/buzz/commit/2b015cb5dcc5f91738736b4f778742c893129e47))
- fix(desktop): omission must not wipe team persona_ids/instructions ([#1985](https://github.com/block/buzz/pull/1985)) ([`1e91fd473`](https://github.com/block/buzz/commit/1e91fd473bb45fdcdb7e34623091e4e9401355dc))
- [codex] Prevent actor-tag UI impersonation ([#1931](https://github.com/block/buzz/pull/1931)) ([`c540ec967`](https://github.com/block/buzz/commit/c540ec967869ef0f4eef90439bf70929fc74f7f6))


## v0.4.9

- Restyle onboarding: branded landing screen, yellow/gradient backgrounds, new starter avatars ([#1982](https://github.com/block/buzz/pull/1982)) ([`831c80c1a`](https://github.com/block/buzz/commit/831c80c1a5d0811c7ac6ae832a95b2fd4ddc4e89))
- Guide CLI installation and subscription sign-in ([#1980](https://github.com/block/buzz/pull/1980)) ([`8d3666c5f`](https://github.com/block/buzz/commit/8d3666c5f8ea07abe65355a1afe7de95ea247441))
- unify channel add + search into one entry point ([#1964](https://github.com/block/buzz/pull/1964)) ([`3dd236eb6`](https://github.com/block/buzz/commit/3dd236eb6fd7c0dd86a9174d2852bc9ae9861912))
- Apply optional relay join policy across join flows ([#1894](https://github.com/block/buzz/pull/1894)) ([`6c2d66757`](https://github.com/block/buzz/commit/6c2d667575cbc372ba42d26134448660fb1d2ee9))
- fix(desktop): preserve relaunch through mesh shutdown ([#1966](https://github.com/block/buzz/pull/1966)) ([`a1626f96c`](https://github.com/block/buzz/commit/a1626f96cea57e415dad72558df04494ab4c2596))
- Persist agent audiences with native inline mentions ([#1949](https://github.com/block/buzz/pull/1949)) ([`19dc33bda`](https://github.com/block/buzz/commit/19dc33bda6e9e7f09703c4efcd721235de644ce9))


## v0.4.8

- fix(desktop): restore default community join option ([#1969](https://github.com/block/buzz/pull/1969)) ([`a84fe13a8`](https://github.com/block/buzz/commit/a84fe13a8c419f3d2d6ff3b577b431629b8e3e07))
- feat(desktop): refine Projects navigation and overview hierarchy ([#1956](https://github.com/block/buzz/pull/1956)) ([`158259d95`](https://github.com/block/buzz/commit/158259d953dd56ec82cdb121b2caf075df573458))
- Log handoff token counts ([#1954](https://github.com/block/buzz/pull/1954)) ([`ca8324d2c`](https://github.com/block/buzz/commit/ca8324d2cf696f35a553b9a561dd608881920b85))
- fix(onboarding): add Welcome finalize skip escape hatch ([#1960](https://github.com/block/buzz/pull/1960)) ([`7ed6422c1`](https://github.com/block/buzz/commit/7ed6422c1a6e26d9c13bd9c0eb5783d8e784d173))
- Welcome channel: Browse channels card, browser sorting, shortcut hint placement ([#1948](https://github.com/block/buzz/pull/1948)) ([`f20d79658`](https://github.com/block/buzz/commit/f20d79658c7106d2d87b4513260a594a647569b7))


## v0.4.7

- feat(desktop): confirm invites with an Opening-invite loading gate before machine onboarding ([#1950](https://github.com/block/buzz/pull/1950)) ([`2510a4b4c`](https://github.com/block/buzz/commit/2510a4b4c91b54db5a3aad852c391f3974f48ebd))
- feat(desktop): remove theme selection from onboarding, default to Buzz + System ([#1947](https://github.com/block/buzz/pull/1947)) ([`2c08271ba`](https://github.com/block/buzz/commit/2c08271ba9f12c87ded0504f0ef028d32ff6a124))
- fix(desktop): sentence-case runtime descriptions in onboarding setup ([#1944](https://github.com/block/buzz/pull/1944)) ([`36c3c4994`](https://github.com/block/buzz/commit/36c3c4994964142a29dccee8d54669f1c3ebdd4d))
- feat(desktop): Projects v3 — overview redesign, activity feed, agent access & sync controls ([#1851](https://github.com/block/buzz/pull/1851)) ([`72a5c388d`](https://github.com/block/buzz/commit/72a5c388d79761b22f8cf7a3c3d41a41e40768cc))
- Rework first-launch community onboarding ([#1936](https://github.com/block/buzz/pull/1936)) ([`8688c0416`](https://github.com/block/buzz/commit/8688c0416d50341627b0533f405469308256cf6a))
- fix(desktop): stop marking users away while active on the machine ([#1942](https://github.com/block/buzz/pull/1942)) ([`e1865cb7f`](https://github.com/block/buzz/commit/e1865cb7f7daef1275328ed684254c531def1eb5))
- fix(desktop): variant-aware connecting gate for first-run handoffs ([#1938](https://github.com/block/buzz/pull/1938)) ([`dd39b6249`](https://github.com/block/buzz/commit/dd39b6249fc497c28cc1cc7111003ab9f8688ce0))
- Redesign agent AI configuration and defaults ([#1935](https://github.com/block/buzz/pull/1935)) ([`2910ea45d`](https://github.com/block/buzz/commit/2910ea45de5672dc07e716864419dea7c1914b26))
- Add Buzz-managed Node runtime for agent installs ([#1930](https://github.com/block/buzz/pull/1930)) ([`0722346b1`](https://github.com/block/buzz/commit/0722346b1f9efb0e4b3bc6c9d9ff038d79b1b282))
- feat(desktop): return nostr binding proof in browser fragment ([#1933](https://github.com/block/buzz/pull/1933)) ([`ccf7f1a2d`](https://github.com/block/buzz/commit/ccf7f1a2d0c3fdafc3f61dd1b88161f271df8cb8))
- Use selected relay for desktop pairing socket ([#1934](https://github.com/block/buzz/pull/1934)) ([`91be66a2d`](https://github.com/block/buzz/commit/91be66a2d76fb176e8f375a224a607f9b52e14df))
- fix(desktop): allow editing built-in agents ([#1928](https://github.com/block/buzz/pull/1928)) ([`202201f3a`](https://github.com/block/buzz/commit/202201f3aa4be3eff22c8ebb408bd51e44c37585))
- fix(desktop): load downloaded mesh runtime on signed macOS builds ([#1932](https://github.com/block/buzz/pull/1932)) ([`32957692e`](https://github.com/block/buzz/commit/32957692eb94ac133c60ac303e4a05ab3142176b))
- feat(media): require auth for relay media reads ([#1926](https://github.com/block/buzz/pull/1926)) ([`f30876285`](https://github.com/block/buzz/commit/f3087628524951de91028c9d263bcd0d0a727fab))
- fix(desktop): remove superseded built-in agents ([#1929](https://github.com/block/buzz/pull/1929)) ([`dbf2cfc21`](https://github.com/block/buzz/commit/dbf2cfc21ef96231f54bc49575605d1dbcea25d3))
- Move agent AI defaults into Settings ([#1919](https://github.com/block/buzz/pull/1919)) ([`3e90dd5dd`](https://github.com/block/buzz/commit/3e90dd5dd8a5f470ba3afa45f80fd594e8a15547))
- feat(desktop): add Honey and Bumble starter agents ([#1925](https://github.com/block/buzz/pull/1925)) ([`335413461`](https://github.com/block/buzz/commit/335413461b5609408af4733d32baaa640223d18d))
- feat(desktop): expand owned team mentions ([#1918](https://github.com/block/buzz/pull/1918)) ([`470458d32`](https://github.com/block/buzz/commit/470458d32561ad03e895e0e34ca9b966e729ab89))
- Fix live agent reactions in Inbox and channels ([#1921](https://github.com/block/buzz/pull/1921)) ([`8212a8723`](https://github.com/block/buzz/commit/8212a872310ced1156328ccc75924f637a78a67a))
- refactor(desktop): unify page/section header hierarchy across Agents and Settings ([#1912](https://github.com/block/buzz/pull/1912)) ([`7fbdacb03`](https://github.com/block/buzz/commit/7fbdacb03f464f341374265bfed0747b207214f6))


## v0.4.6

- Polish agent and team sharing and snapshots ([#1852](https://github.com/block/buzz/pull/1852)) ([`2f3fa586f`](https://github.com/block/buzz/commit/2f3fa586f3bd12b26b8a9dc69a424d5c28b1e27e))
- fix(desktop): filter punctuation from avatar initials ([#1904](https://github.com/block/buzz/pull/1904)) ([`63e099709`](https://github.com/block/buzz/commit/63e099709667669be41426e7144a4a9057f8537b))
- fix(desktop): preserve snapshot attachment URLs ([#1905](https://github.com/block/buzz/pull/1905)) ([`1ab7bbfe4`](https://github.com/block/buzz/commit/1ab7bbfe453d10cfd3522c358e5ed18931f1e1ec))
- Chat-first agent creation via portable buzz CLI drafts ([#1878](https://github.com/block/buzz/pull/1878)) ([`54e174b5e`](https://github.com/block/buzz/commit/54e174b5e1424c13fe7451c86a418ba8b9c01f1c))
- feat(desktop): enable community rail by default ([#1902](https://github.com/block/buzz/pull/1902)) ([`55a46b063`](https://github.com/block/buzz/commit/55a46b063e291862c034d4ce685b3d3a92804951))
- feat(desktop): redesign Nostr bind verification flow ([#1850](https://github.com/block/buzz/pull/1850)) ([`abfe78aaf`](https://github.com/block/buzz/commit/abfe78aafb131c9100f33ed361850fb9cfeb0881))
- fix(desktop): simplify sign out settings row ([#1903](https://github.com/block/buzz/pull/1903)) ([`3e8dae7ff`](https://github.com/block/buzz/commit/3e8dae7ff3c73b1b2b1ae26d5a4bed5db17cb269))
- feat(desktop): Send feedback modal + profile presence chip ([#1756](https://github.com/block/buzz/pull/1756)) ([`f5c33d333`](https://github.com/block/buzz/commit/f5c33d3335fc3fa7e304bb51a9931d4cc741d255))
- fix(desktop): seed timeline virtualization row heights ([#1887](https://github.com/block/buzz/pull/1887)) ([`406cf7911`](https://github.com/block/buzz/commit/406cf7911ee1f335fdf0be131ee057ea2c455037))
- fix(desktop): dedupe react-dismissable-layer to stop modal menu → dialog freezes ([#1899](https://github.com/block/buzz/pull/1899)) ([`366567ac2`](https://github.com/block/buzz/commit/366567ac20b4786c1ca8e754ac5c45f0e7d59122))
- fix(desktop): clean up Remind me later dialog footer and loading state ([#1898](https://github.com/block/buzz/pull/1898)) ([`40fe65095`](https://github.com/block/buzz/commit/40fe6509583e93054eab04906aacde834c5fcc68))
- fix(desktop): let sidebar action card description wrap over multiple lines ([#1891](https://github.com/block/buzz/pull/1891)) ([`b36ee3e30`](https://github.com/block/buzz/commit/b36ee3e30203b4cc4ec8056ae4d9c57acf26891e))
- mesh: upgrade runtime, enforce membership, add shared compute provider ([#1656](https://github.com/block/buzz/pull/1656)) ([`54638ff4b`](https://github.com/block/buzz/commit/54638ff4bb5af2d3d3759b44118b43052f814bb1))


## v0.4.5

- test(desktop): remove stale collapsed home chrome check ([#1871](https://github.com/block/buzz/pull/1871)) ([`395a3f149`](https://github.com/block/buzz/commit/395a3f1492807b065f8edbee9bb9c8d51018095a))
- fix(desktop): hide message action for inaccessible agents ([#1883](https://github.com/block/buzz/pull/1883)) ([`8ce9e7914`](https://github.com/block/buzz/commit/8ce9e791453a262f21e675d3f5d27ace22a09e9b))
- feat(desktop): add API key field, global default indicators, and collapsed advanced ([#1875](https://github.com/block/buzz/pull/1875)) ([`1742dce7b`](https://github.com/block/buzz/commit/1742dce7b25676e91631ce4056bdd2bbc05944ab))
- fix(desktop): foreground app for deep links ([#1880](https://github.com/block/buzz/pull/1880)) ([`a26168f55`](https://github.com/block/buzz/commit/a26168f553fd8bfce8b5df88915e76ff2d7004ee))
- chore(desktop): replace setup ? operators with clean exit ([#1882](https://github.com/block/buzz/pull/1882)) ([`4c106d878`](https://github.com/block/buzz/commit/4c106d878564e3f5b4def92f4667fe95dec015f4))
- fix(desktop): remediate all onboarding dead paths (D1-D7) ([#1856](https://github.com/block/buzz/pull/1856)) ([`5bb619295`](https://github.com/block/buzz/commit/5bb6192951bd57c535a34cd835aba20714f408b6))


## v0.4.4

- fix(desktop): allow staged updater commands ([#1870](https://github.com/block/buzz/pull/1870)) ([`109c2c526`](https://github.com/block/buzz/commit/109c2c52641c5edd50890bffef9a236a10ea6106))
- feat(teams): unify team model, snapshot sharing, and PNG memory parity ([#1846](https://github.com/block/buzz/pull/1846)) ([`448baeef7`](https://github.com/block/buzz/commit/448baeef770478411678e2f0be1729f04b6800d7))
- fix(desktop): always start agents and hide unavailable mesh ([#1860](https://github.com/block/buzz/pull/1860)) ([`37bc962cb`](https://github.com/block/buzz/commit/37bc962cb8fb60857337bbfb97eb4d31c359de15))
- Relay mesh: cross-pod tunnel + huddle transport (buzz-relay-mesh) ([#1670](https://github.com/block/buzz/pull/1670)) ([`ccb021d71`](https://github.com/block/buzz/commit/ccb021d71339009aabedc383c8f3d8e5c23e1e42))
- feat(desktop): add Sign Out to Settings to reset and relaunch ([#1842](https://github.com/block/buzz/pull/1842)) ([`cf9723796`](https://github.com/block/buzz/commit/cf97237965a5ed9870f1c1f73ce9952e963b418b))


## v0.4.3

- Stabilize persona and stream end-to-end tests ([#1865](https://github.com/block/buzz/pull/1865)) ([`916de4e4d`](https://github.com/block/buzz/commit/916de4e4dbbf9299695a91defa6e3802b1820256))
- refactor(clients): standardize product naming on community ([#1858](https://github.com/block/buzz/pull/1858)) ([`3e76481a1`](https://github.com/block/buzz/commit/3e76481a149bb3de459298cc989505c470c2372c))
- fix(desktop): retain live rows after window exhaustion ([#1810](https://github.com/block/buzz/pull/1810)) ([`fa6f4819b`](https://github.com/block/buzz/commit/fa6f4819b3468a5a3725356dd71463bb37cc30c7))
- Add private product feedback sidecar ([#1857](https://github.com/block/buzz/pull/1857)) ([`af190c93e`](https://github.com/block/buzz/commit/af190c93e1048af64c3fbfb3831c689cb703997c))
- fix(desktop): resolve Doctor install shell and command detection on Windows ([#1854](https://github.com/block/buzz/pull/1854)) ([`1aec7ea7a`](https://github.com/block/buzz/commit/1aec7ea7a798c8bdb131a802dcd3d2ea0fb8692a))
- fix(desktop): navigate to channel from inbox thread header ([#1847](https://github.com/block/buzz/pull/1847)) ([`ff11f26bc`](https://github.com/block/buzz/commit/ff11f26bcdc8e107d7d04f5682b2d17f586536d6))
- feat(desktop): surface needsRestart badge on live UI surfaces ([#1853](https://github.com/block/buzz/pull/1853)) ([`b1d323c4e`](https://github.com/block/buzz/commit/b1d323c4e7ddbabbe52a868f3699a523bb6c010e))
- fix(desktop): prevent menu-to-dialog UI lockups ([#1839](https://github.com/block/buzz/pull/1839)) ([`7f81d93eb`](https://github.com/block/buzz/commit/7f81d93eb1cf6a10c84d948d91c45a1fd4778060))
- feat(desktop): add conversation-style DM composer ([#1768](https://github.com/block/buzz/pull/1768)) ([`bb9d9b0c0`](https://github.com/block/buzz/commit/bb9d9b0c049b9561458414b24eff157c6bbbd446))
- feat(push): add public APNs gateway ([#1770](https://github.com/block/buzz/pull/1770)) ([`1c006822e`](https://github.com/block/buzz/commit/1c006822e4484d68e33fce14f9139c2f70ce9d66))
- fix(desktop): stabilize agent identity restore ([#1831](https://github.com/block/buzz/pull/1831)) ([`3370cd083`](https://github.com/block/buzz/commit/3370cd0836e458f519cf90fe6299279cde6c7463))
- fix(observer): align scroll-anchor ids with transcript display-block keys ([#1849](https://github.com/block/buzz/pull/1849)) ([`1df4a4e77`](https://github.com/block/buzz/commit/1df4a4e77da732cadad91738bc0271a926c22040))
- [codex] Add view activity label to agent popover ([#1748](https://github.com/block/buzz/pull/1748)) ([`458c7f915`](https://github.com/block/buzz/commit/458c7f9154fc9580a766117cad0e3e378ffe5ac0))
- ci(desktop): surface flaky E2E tests instead of retry-masking them ([#1838](https://github.com/block/buzz/pull/1838)) ([`a65340630`](https://github.com/block/buzz/commit/a653406309aa6dab8122cf3e84ddeca0c87013be))
- fix(desktop): treat channel creator as member before 39002 provisioning ([#1830](https://github.com/block/buzz/pull/1830)) ([`7e62a25af`](https://github.com/block/buzz/commit/7e62a25af0bc71df17de806673ed02514e556e6e))


## v0.4.2

- fix(desktop): unify observer feed scroll onto useAnchoredScroll ([#1825](https://github.com/block/buzz/pull/1825)) ([`d75c2e913`](https://github.com/block/buzz/commit/d75c2e913eef763e8ea2e748c58353c3a22d544d))
- fix(desktop): sync sidebar update card copy with global installing state ([#1827](https://github.com/block/buzz/pull/1827)) ([`b63a2e423`](https://github.com/block/buzz/commit/b63a2e4231d27f61eae952d2febab36489e773a5))
- fix(desktop): parse codex ACP plan entries[] into checklist ([#1824](https://github.com/block/buzz/pull/1824)) ([`34dcd13d5`](https://github.com/block/buzz/commit/34dcd13d559b6c606312fca1bbc2c52310b35603))
- fix(desktop): resolve Git Bash for Windows shell tool via PATH/git/registry fallback ([#1821](https://github.com/block/buzz/pull/1821)) ([`020ac7f40`](https://github.com/block/buzz/commit/020ac7f405c24f72e390b605571fb65e2481612c))
- feat(desktop): show read-only MCP server config ([#1780](https://github.com/block/buzz/pull/1780)) ([`51ee1c473`](https://github.com/block/buzz/commit/51ee1c473dae018499e42106086c359c19108137))
- fix(desktop): require user action before applying updates ([#1820](https://github.com/block/buzz/pull/1820)) ([`deed64a14`](https://github.com/block/buzz/commit/deed64a14f1ec39a1242bed46711e44d77a3086e))
- fix(desktop): preserve selected inbox rows through reflow ([#1817](https://github.com/block/buzz/pull/1817)) ([`c06ddcf14`](https://github.com/block/buzz/commit/c06ddcf14b81834a59a4eedf020ccbe8ca72dfd7))
- fix(snapshot): send Buzz shares as PNG avatar cards ([#1811](https://github.com/block/buzz/pull/1811)) ([`a6cfd6551`](https://github.com/block/buzz/commit/a6cfd65511ee06f9073f2d5405e2242a9e2ff4ef))
- test(desktop): consolidate and stabilize scroll coverage ([#1815](https://github.com/block/buzz/pull/1815)) ([`d41b4c390`](https://github.com/block/buzz/commit/d41b4c390522d63a0a4e1b3e8a93fe9c84289b2a))
- fix(buzz-agent): support max effort for gpt-5.6 family ([#1806](https://github.com/block/buzz/pull/1806)) ([`e34e0974a`](https://github.com/block/buzz/commit/e34e0974a149a2b776e779e5798cc4252cc5030c))
- fix(desktop): widen probe test timing margins for parallel pre-push ([#1812](https://github.com/block/buzz/pull/1812)) ([`fac79215a`](https://github.com/block/buzz/commit/fac79215a4cdc8289cc94282641e69d822d419d6))
- fix(desktop): keep pairing tests after production items ([#1807](https://github.com/block/buzz/pull/1807)) ([`68909629f`](https://github.com/block/buzz/commit/68909629f96a95797a627b19e76e03bcbd6983de))
- chore(deps): update rust crate nostr to v0.44.4 ([#1789](https://github.com/block/buzz/pull/1789)) ([`f1f5002a9`](https://github.com/block/buzz/commit/f1f5002a970bb651173ef49027e9609b903e3fa3))
- Add optional standalone pairing relay to Helm chart ([#1799](https://github.com/block/buzz/pull/1799)) ([`9b47c8548`](https://github.com/block/buzz/commit/9b47c8548fd061fbb806ea8b9ddee831c19cf80e))
- chore(ci): lint desktop Tauri crate in CI ([#1801](https://github.com/block/buzz/pull/1801)) ([`bea507d8a`](https://github.com/block/buzz/commit/bea507d8aa611d8d0be893bac1339dad50162d7b))
- fix(desktop): preserve live events and window order across channel refreshes ([#1802](https://github.com/block/buzz/pull/1802)) ([`e09e94ab8`](https://github.com/block/buzz/commit/e09e94ab88543ea0e0c6411092852c1c98187feb))
- fix(desktop): restore macOS navigation chrome alignment ([#1797](https://github.com/block/buzz/pull/1797)) ([`109800fe5`](https://github.com/block/buzz/commit/109800fe5a67c560e3ffb750e7df28177c03f509))
- fix(desktop): omit invalid snapshot imeta thumbnails ([#1800](https://github.com/block/buzz/pull/1800)) ([`b32f2c08f`](https://github.com/block/buzz/commit/b32f2c08f7cdbad1bcc3b6a73711c2e7a21c71bd))
- fix(desktop): centralize known-agent trust set in useKnownAgentPubkeys ([#1703](https://github.com/block/buzz/pull/1703)) ([`095f3c728`](https://github.com/block/buzz/commit/095f3c7280153394bf2a8215b1355ef734af9ee6))
- chore(deps): update all non-major dependencies ([#1778](https://github.com/block/buzz/pull/1778)) ([`221aceb1c`](https://github.com/block/buzz/commit/221aceb1c2f794db0bf2050d1cc66fe8d2db9cec))
- fix(desktop): pass augmented PATH to codex-acp version probe ([#1794](https://github.com/block/buzz/pull/1794)) ([`10ae66893`](https://github.com/block/buzz/commit/10ae66893c8b454255298d0eab40bfd47a46041c))
- feat(desktop): add team snapshot sharing commands ([#1790](https://github.com/block/buzz/pull/1790)) ([`150514f66`](https://github.com/block/buzz/commit/150514f665c5aeebb83f9c42c17e793d88ee6b9f))
- feat(managed_agents): add buzz-team-snapshot v1 codec ([#1784](https://github.com/block/buzz/pull/1784)) ([`aceb5e045`](https://github.com/block/buzz/commit/aceb5e0456c19761f1210585be520fa291ef903c))


## v0.4.1

- feat(desktop): auto-restart setup-mode agents after adapter install, badge drift fallback ([#1786](https://github.com/block/buzz/pull/1786)) ([`11a286b9a`](https://github.com/block/buzz/commit/11a286b9a4284f4f80943d1321f86fa41c472dcf))
- Retain a stable message window in channel timelines ([#1698](https://github.com/block/buzz/pull/1698)) ([`b0ad2b66d`](https://github.com/block/buzz/commit/b0ad2b66ddb2d91bcee1473050d21f27e2a38eac))
- refactor(desktop): remove legacy persona-card flows ([#1781](https://github.com/block/buzz/pull/1781)) ([`1fa91f569`](https://github.com/block/buzz/commit/1fa91f569114cfa1d22951311fe80cef0def36d7))
- Serialize managed agent PATH tests ([#1777](https://github.com/block/buzz/pull/1777)) ([`1fc7488ea`](https://github.com/block/buzz/commit/1fc7488ea58accf8bd96afe94cd93ee54174df00))
- fix(desktop): stable inbox selection and scroll/draft preservation ([#1760](https://github.com/block/buzz/pull/1760)) ([`7e3252ea3`](https://github.com/block/buzz/commit/7e3252ea3e6b5c6c0db2e2a36c839d9cc2700c89))
- feat(desktop): add buzz-agent-snapshot v1 export, import, sender-native send, and recipient card/import ([#1753](https://github.com/block/buzz/pull/1753)) ([`1580046b3`](https://github.com/block/buzz/commit/1580046b3c2c3d208e84fb240467454b8f3813f0))
- refactor(desktop): remove vestigial MCP toolsets config ([#1776](https://github.com/block/buzz/pull/1776)) ([`dfec75b3c`](https://github.com/block/buzz/commit/dfec75b3c0b8080529e4d9089d4ed80e3902aaed))
- feat(desktop): add key backup and agent defaults to onboarding, make avatar optional ([#1767](https://github.com/block/buzz/pull/1767)) ([`d2e87e1ca`](https://github.com/block/buzz/commit/d2e87e1ca71368312576b7f492818a50042e61e1))
- fix(desktop): correct provider and model handling in agent config dialogs ([#1764](https://github.com/block/buzz/pull/1764)) ([`f3319dd11`](https://github.com/block/buzz/commit/f3319dd11102bfd52e1ec3065a4d9272a03075ec))
- fix(desktop): cascade persona deletes and restart agents on global config save ([#1766](https://github.com/block/buzz/pull/1766)) ([`7c346d7f8`](https://github.com/block/buzz/commit/7c346d7f85e7fcb64999b502306f7895d37fb42a))
- fix(desktop): make doctor installs retryable with per-runtime progress and auth status ([#1765](https://github.com/block/buzz/pull/1765)) ([`25bb71475`](https://github.com/block/buzz/commit/25bb7147523f27a88f1bfece0310889442699c5f))
- fix(codex): swap to @agentclientprotocol/codex-acp 1.x + detect outdated adapter ([#1750](https://github.com/block/buzz/pull/1750)) ([`6d1a77e49`](https://github.com/block/buzz/commit/6d1a77e49255f18434593c9c7daa811f5c76db0b))
- fix(desktop): show thread replies loader ([#1773](https://github.com/block/buzz/pull/1773)) ([`e07e02c0e`](https://github.com/block/buzz/commit/e07e02c0eb76376b96e0a9d5fe27009111430845))
- fix(desktop): restore multi-image mosaic galleries ([#1769](https://github.com/block/buzz/pull/1769)) ([`2c41e9e6b`](https://github.com/block/buzz/commit/2c41e9e6b836fad57da981ec9329fe51e5796085))
- Group channel membership events ([#1713](https://github.com/block/buzz/pull/1713)) ([`5dc70bd0b`](https://github.com/block/buzz/commit/5dc70bd0b8561ce8aa24338903167e62d6035d10))
- Fix desktop launch motion and reaction spacing ([#1717](https://github.com/block/buzz/pull/1717)) ([`89e2e2e4a`](https://github.com/block/buzz/commit/89e2e2e4adf4d062995aed46b32f4935c6992513))
- fix(desktop): align sidebar search across themes ([#1712](https://github.com/block/buzz/pull/1712)) ([`53c177e34`](https://github.com/block/buzz/commit/53c177e341d4f9e7b254b7c83af39380e9b10242))
- fix(desktop): Buzz theme flicker, white bar & accent-picker motion ([#1681](https://github.com/block/buzz/pull/1681)) ([`5e15a588e`](https://github.com/block/buzz/commit/5e15a588e072198b922fb3793a48d9c98146fc45))
- feat(desktop): surface Team Instructions as distinct observer section ([#1759](https://github.com/block/buzz/pull/1759)) ([`56d822dc2`](https://github.com/block/buzz/commit/56d822dc2c9b78df50fad7928a8d088b7618e671))
- fix(desktop): separate system prompt from turn context ([#1754](https://github.com/block/buzz/pull/1754)) ([`84ec63aa2`](https://github.com/block/buzz/commit/84ec63aa24d14553206dde5dbfc61807755313b8))
- fix(desktop): preserve archived observer history ([#1752](https://github.com/block/buzz/pull/1752)) ([`51234bd89`](https://github.com/block/buzz/commit/51234bd89f1dffcd82bae45428094cff35c4081e))
- fix(desktop): surface codex config-parse failures and -32603 internal errors clearly ([#1745](https://github.com/block/buzz/pull/1745)) ([`ec3762552`](https://github.com/block/buzz/commit/ec37625522ce1847e2684da2a477ca742255fbf1))
- fix(desktop): fix workspace rail badge disappearance and persist mark-as-unread ([#1747](https://github.com/block/buzz/pull/1747)) ([`e8df6ba69`](https://github.com/block/buzz/commit/e8df6ba694703a26963524433e0e3a3f6ad252f1))
- chore(desktop): remove container-only npm-preflight E2E harness ([#1749](https://github.com/block/buzz/pull/1749)) ([`9895b4d39`](https://github.com/block/buzz/commit/9895b4d390591a5adfc368bbc97f53989463c8ef))
- fix(desktop): preflight npm prefix writability in doctor installs ([#1732](https://github.com/block/buzz/pull/1732)) ([`58aee8f91`](https://github.com/block/buzz/commit/58aee8f91394a97b68902f61a116696ed8ca3e1f))
- fix(workspaces): exclude muted channels and unfollowed threads from rail unread badge ([#1738](https://github.com/block/buzz/pull/1738)) ([`ea1c75b0d`](https://github.com/block/buzz/commit/ea1c75b0d2bf954efe3816b303a9972e26ff893a))
- feat(desktop): add explicit Edit avatar CTA in agent instance edit dialog ([#1736](https://github.com/block/buzz/pull/1736)) ([`26966aace`](https://github.com/block/buzz/commit/26966aacec1d8ca58ac5951818e55249e63506a3))
- fix(observer): order system prompt below current-session divider on restart ([#1734](https://github.com/block/buzz/pull/1734)) ([`67a2d047e`](https://github.com/block/buzz/commit/67a2d047eb866dff3faae9f96b0a4f478e5e2a8b))


## v0.4.0

- feat(desktop): default observer archive on for dev-nest builds ([#1726](https://github.com/block/buzz/pull/1726)) ([`d9e4edbbf`](https://github.com/block/buzz/commit/d9e4edbbf54996df22c59afdaaaaf571b4e3a5c6))
- feat(desktop): thread baked build env into global agent config UI ([#1722](https://github.com/block/buzz/pull/1722)) ([`2f2ad409a`](https://github.com/block/buzz/commit/2f2ad409a76606dd9c54872dc518b0ecb72756af))
- feat: relay invite links (mint + claim + landing page + deep link) ([#1668](https://github.com/block/buzz/pull/1668)) ([`2e529aab7`](https://github.com/block/buzz/commit/2e529aab759a18c1bb81e447f3696fe99db53a27))
- fix(desktop): pass GlobalAgentConfig to spawn_config_hash in tests ([#1721](https://github.com/block/buzz/pull/1721)) ([`b97777350`](https://github.com/block/buzz/commit/b97777350dec375a1fe444891c8e10f9cee74d0c))
- feat(desktop): global agent config defaults with readiness/spawn parity ([#1448](https://github.com/block/buzz/pull/1448)) ([`77c08365e`](https://github.com/block/buzz/commit/77c08365e4a66c0cc8731b66280ef029500bbfcf))
- fix(desktop): remove "Remove all stopped" bulk action from agents menu ([#1720](https://github.com/block/buzz/pull/1720)) ([`651efe8d9`](https://github.com/block/buzz/commit/651efe8d95d11a591d8421973bf4352e36d69a3f))
- fix(desktop): retire single-member built-in Fizz team ([#1718](https://github.com/block/buzz/pull/1718)) ([`f12bd3689`](https://github.com/block/buzz/commit/f12bd36893ab37f15261d47d2412732089e1645f))
- fix(desktop): hide relay mesh option when mesh-llm feature is absent ([#1719](https://github.com/block/buzz/pull/1719)) ([`f0b8f806f`](https://github.com/block/buzz/commit/f0b8f806fde83b2ea8fc048e1076f4338e445545))
- fix(desktop): propagate agent definition edits to instances and remove dead config knobs ([#1715](https://github.com/block/buzz/pull/1715)) ([`1f48860b7`](https://github.com/block/buzz/commit/1f48860b7b5454a07f4f066ccca665dc7ee26b20))
- perf(desktop): thread panel rebuilt on every render — fix memo + add typing-latency benchmark ([#1652](https://github.com/block/buzz/pull/1652)) ([`9b2f0c8cb`](https://github.com/block/buzz/commit/9b2f0c8cb574f37eb38033f849b8420d15a63716))
- fix(drafts): stop clearing drafts on workspace switch and hide sent section ([#1708](https://github.com/block/buzz/pull/1708)) ([`e2d104f66`](https://github.com/block/buzz/commit/e2d104f669468358ea917f1d7fa2dc9fc5958764))
- fix(inbox): reply at same thread level as inbox item ([#1714](https://github.com/block/buzz/pull/1714)) ([`177fc30be`](https://github.com/block/buzz/commit/177fc30bea397b9422c25eb467c33496ebba9cbe))
- fix(desktop): make @mention clicks reliably open the profile panel ([#1705](https://github.com/block/buzz/pull/1705)) ([`5f0d8309d`](https://github.com/block/buzz/commit/5f0d8309d266262965489eedea516f95a02fd410))
- fix(desktop): walk ancestor chain in orphan sweep exemption ([#1711](https://github.com/block/buzz/pull/1711)) ([`7fb215c4f`](https://github.com/block/buzz/commit/7fb215c4fc29ecf2311f84d8f1f93b0301a582fa))
- fix(desktop): stop showing /dev/null in diff labels, keep type badge visible ([#1704](https://github.com/block/buzz/pull/1704)) ([`868c9ad05`](https://github.com/block/buzz/commit/868c9ad05555dbc11d5c1ceece1e0a310bc3ab4a))
- perf(desktop): stop re-rendering the app shell on every keystroke; fix two latent cache races it unmasked ([#1692](https://github.com/block/buzz/pull/1692)) ([`129c91a2b`](https://github.com/block/buzz/commit/129c91a2ba0017b20b65cbad8099269a383b46a4))
- fix(observer): widen live subscription lookback to capture session/prompt ([#1710](https://github.com/block/buzz/pull/1710)) ([`715df2537`](https://github.com/block/buzz/commit/715df2537594da42c5b19406805c6c410c86a15f))
- test(desktop): stabilize custom emoji e2e ([#1685](https://github.com/block/buzz/pull/1685)) ([`20c40d176`](https://github.com/block/buzz/commit/20c40d1769438a1b0fdcd032b3dcfdc79a683898))
- fix(observer): use lifecycle item title as verb instead of hardcoded "Status" ([#1709](https://github.com/block/buzz/pull/1709)) ([`95bc640a5`](https://github.com/block/buzz/commit/95bc640a539c87c1fe0560d1dfa960b653bb96a1))
- chore(desktop): remove dead archive-migration and keyring load_readonly helpers ([#1701](https://github.com/block/buzz/pull/1701)) ([`389a956e0`](https://github.com/block/buzz/commit/389a956e02e85b1a3379df384a14c6d9c449d3c6))
- fix(desktop): stop losing imported identity on every launch ([#1568](https://github.com/block/buzz/pull/1568)) ([`e15f06857`](https://github.com/block/buzz/commit/e15f0685718bcba6d0947a8286ab2deeab0137ef))
- fix(desktop): use augmented PATH for readiness probes ([#1613](https://github.com/block/buzz/pull/1613)) ([`94ec2c2bc`](https://github.com/block/buzz/commit/94ec2c2bc6f5505dbbbd4783c41f3bb20d5a642c))
- fix(desktop): isolate dev keyring from production and make BUZZ_SHARE_IDENTITY keyring-aware ([#1680](https://github.com/block/buzz/pull/1680)) ([`e5d215e3c`](https://github.com/block/buzz/commit/e5d215e3c2f4939fe4d0e1f09c8f9a265f818385))
- fix(desktop): migrate Databricks V1→V2 records at boot and fix readiness gate ([#1686](https://github.com/block/buzz/pull/1686)) ([`1f2afce29`](https://github.com/block/buzz/commit/1f2afce29d46ec09f63d467af4cacdd479a9a336))
- B5 residue: inbound-event security hardening + persona→agent copy sweep ([#1688](https://github.com/block/buzz/pull/1688)) ([`f0e65589a`](https://github.com/block/buzz/commit/f0e65589a9a68cd19457e1196b8eb4db68217a3b))
- fix(desktop): paginate complete channel directory ([#1690](https://github.com/block/buzz/pull/1690)) ([`b41cf3ffc`](https://github.com/block/buzz/commit/b41cf3ffc6d232dc1199f7563bd3c4da8571bfe1))
- fix: flush debounce on Tab/Enter to prevent stale autocomplete ([#1661](https://github.com/block/buzz/pull/1661)) ([`841282141`](https://github.com/block/buzz/commit/841282141b5feb728dd111c9163cd0060f9bbb86))
- fix(desktop): preserve agent timer state across workspace switches ([#1679](https://github.com/block/buzz/pull/1679)) ([`46613dcce`](https://github.com/block/buzz/commit/46613dccec89b064b2ba61d65b191c155a5aac3f))
- fix(desktop): honor macOS title-bar double-click preference ([#1674](https://github.com/block/buzz/pull/1674)) ([`6b2147a5d`](https://github.com/block/buzz/commit/6b2147a5d18d70a3a46d2484d746de3506f8138d))
- feat(desktop): projects overview v2 — aggregate rail, PR review flow, commit detail, and straighter layout ([#1677](https://github.com/block/buzz/pull/1677)) ([`58fe9388c`](https://github.com/block/buzz/commit/58fe9388c81ace47001cba55f29afb76f288f3e0))
- B5: one create path — behavioral quad activation, backfill, menu collapse, legacy dialog deletion ([#1667](https://github.com/block/buzz/pull/1667)) ([`3a7e40027`](https://github.com/block/buzz/commit/3a7e4002787ea3e8f0f91a62fdbd2f1eb3e7a49f))
- Polish Buzz sidebar theme ([#1671](https://github.com/block/buzz/pull/1671)) ([`cdba2a08e`](https://github.com/block/buzz/commit/cdba2a08e1645f63e71965771cfbbf4a1437f169))
- fix(agents): harden structured error-code classification edge cases ([#1663](https://github.com/block/buzz/pull/1663)) ([`fc8a17445`](https://github.com/block/buzz/commit/fc8a17445a5712d0a1456684d78952edf41cf5ae))
- feat: add deeplink nostr identity binding flow ([#1648](https://github.com/block/buzz/pull/1648)) ([`cecd03142`](https://github.com/block/buzz/commit/cecd031428358d7b3aa0326ccca74649bd928231))
- perf(desktop): cache parsed markdown across channel-switch remounts ([#1635](https://github.com/block/buzz/pull/1635)) ([`2b3246934`](https://github.com/block/buzz/commit/2b324693404623b2fa2e53ab6ec570977526ace3))
- fix(agents): surface provider errors structurally instead of raw dumps ([#1653](https://github.com/block/buzz/pull/1653)) ([`9b244ff22`](https://github.com/block/buzz/commit/9b244ff22b2fdab1d4aa20fcf661d22aa0cf84aa))
- feat(relay,desktop): canonicalize agent definitions on kind:30175 (Phase 2) ([#1655](https://github.com/block/buzz/pull/1655)) ([`b2c63291f`](https://github.com/block/buzz/commit/b2c63291f1e432951f49c57bd1ec65812870f515))
- fix(desktop): scope observer feed by channel and add session-boundary dividers ([#1634](https://github.com/block/buzz/pull/1634)) ([`fd5d04de0`](https://github.com/block/buzz/commit/fd5d04de0593ee8c335b57bd2d248c638b055ade))
- feat(config-nudge): distinguish install/auth state in cli_login nudge cards ([#1633](https://github.com/block/buzz/pull/1633)) ([`4f1a487ab`](https://github.com/block/buzz/commit/4f1a487ab49531d4ebe284fd3cee689c13651c7f))
- fix(desktop): async-ify the auto-restart setter after the perf-sweep merge collision ([#1651](https://github.com/block/buzz/pull/1651)) ([`dc7ececdc`](https://github.com/block/buzz/commit/dc7ececdccd8ed5675f1560a5319518ade6b5e8e))
- feat(desktop): auto-restart agents on config change (Chunk F) ([#1649](https://github.com/block/buzz/pull/1649)) ([`a415f0f31`](https://github.com/block/buzz/commit/a415f0f3109fae970d266474fec64f16bab53b33))
- perf(desktop): GUI performance sweep — async offload, poll reduction, render stabilization ([#1641](https://github.com/block/buzz/pull/1641)) ([`2cc0eb539`](https://github.com/block/buzz/commit/2cc0eb53968ec6281be0e6778a06ea18e044395e))
- fix: user search returns zero results for partially typed names ([#1603](https://github.com/block/buzz/pull/1603)) ([`11608ef67`](https://github.com/block/buzz/commit/11608ef675ef11c375b2ac9c04bdcc287affeb46))
- copy(desktop): finish the persona→agent rename in rendered UI (B4.1) ([#1647](https://github.com/block/buzz/pull/1647)) ([`577fc8df5`](https://github.com/block/buzz/commit/577fc8df5254f8fc1c5ec260d22f64f8786c6244))
- Community moderation UI: integration branch (data layer + member surface base) ([#1617](https://github.com/block/buzz/pull/1617)) ([`c9c868b25`](https://github.com/block/buzz/commit/c9c868b250e1dc8f6019abaaa6ae2c203294dfc4))
- Community moderation Phase 1: reports, bans/timeouts, audit, tombstones, relay-DM notices ([#1616](https://github.com/block/buzz/pull/1616)) ([`863aeb79f`](https://github.com/block/buzz/commit/863aeb79f35a43222bd5de3901ab06220948fb9d))
- copy(desktop): the word "persona" leaves the UI (Phase 1B.4) ([#1646](https://github.com/block/buzz/pull/1646)) ([`41b187f93`](https://github.com/block/buzz/commit/41b187f93646403689311d5f5bfcbc4cc1bf4927))
- feat(desktop): flapping bee on the setup loading screen ([#1631](https://github.com/block/buzz/pull/1631)) ([`c145037aa`](https://github.com/block/buzz/commit/c145037aad121bb9f6fd6e9068ff3cf77fb86a48))
- fix(desktop): make ⌘K open the composer link editor for selections ([#1644](https://github.com/block/buzz/pull/1644)) ([`3e982fd37`](https://github.com/block/buzz/commit/3e982fd371b97aebf39a3e418e323e6c0de7b665))
- fix(linux): make AppImage work on Mesa 25+ / GLib 2.88 distros ([#1567](https://github.com/block/buzz/pull/1567)) ([`563dab92e`](https://github.com/block/buzz/commit/563dab92e4f07b87db0c4156a20b0041c60b561c))
- fix(desktop): add unread dot to workspace rail for channel unreads ([#1637](https://github.com/block/buzz/pull/1637)) ([`f968601ef`](https://github.com/block/buzz/commit/f968601ef6208ce411989617d15b2f3592a51665))
- refactor(desktop): converge the three definition→instance mappings (Phase 1B.3.5) ([#1645](https://github.com/block/buzz/pull/1645)) ([`9683ef807`](https://github.com/block/buzz/commit/9683ef807919f4d04f960280bff3b9d99da12af3))
- Hide non-invocable member agents from @-mention autocomplete ([#1611](https://github.com/block/buzz/pull/1611)) ([`f929aa09e`](https://github.com/block/buzz/commit/f929aa09e685eece9ff710c7506b07834df1f00e))
- fix(desktop): clamp HDR avatar images to SDR range ([#1642](https://github.com/block/buzz/pull/1642)) ([`fa816d5bf`](https://github.com/block/buzz/commit/fa816d5bfade9eb84e93b720bfc55e6af4e623f4))
- refactor(desktop): definition-edit routes through AgentDialog — single dialog entry point (Phase 1B.3c) ([#1643](https://github.com/block/buzz/pull/1643)) ([`aa17294ad`](https://github.com/block/buzz/commit/aa17294ad07c0fa64b94502e9e9a07aa9bd5c70d))
- fix(desktop): refresh persona env vars on respawn — record.env_vars is overrides-only ([#1640](https://github.com/block/buzz/pull/1640)) ([`79a057534`](https://github.com/block/buzz/commit/79a0575342016b8aebeff322e84c9cea0bf76443))
- refactor(desktop): re-host instance edit as AgentInstanceEditDialog behind AgentDialog (Phase 1B.3b) ([#1639](https://github.com/block/buzz/pull/1639)) ([`8af18ee32`](https://github.com/block/buzz/commit/8af18ee32515db20ec5b4d3d430a49f702209cdc))
- feat(desktop): show full URL tooltip on masked link hover ([#1625](https://github.com/block/buzz/pull/1625)) ([`01c6b8566`](https://github.com/block/buzz/commit/01c6b8566e7d6ccd291c4a244285927476196b6d))
- fix(desktop): paint Buzz gradient on the workspace rail ([#1636](https://github.com/block/buzz/pull/1636)) ([`2f1cd6771`](https://github.com/block/buzz/commit/2f1cd677147b5d07979cef112c728a96a4dff4f1))
- refactor(desktop): single-home the definition form as AgentDefinitionDialog (Phase 1B.3a) ([#1627](https://github.com/block/buzz/pull/1627)) ([`48bd8abfe`](https://github.com/block/buzz/commit/48bd8abfeb165f23f95d456be91b45c4b698a0df))
- feat(desktop): unified AgentDialog create entry point with create intents (Phase 1B.2) ([#1626](https://github.com/block/buzz/pull/1626)) ([`6f38db680`](https://github.com/block/buzz/commit/6f38db68039300d7f92122b9d7631b5a4bbe5e53))
- Update DMG background and label size ([#1629](https://github.com/block/buzz/pull/1629)) ([`5b1a53ceb`](https://github.com/block/buzz/commit/5b1a53ceba2c6d9196a52b478c1f581c4d42b8df))
- test(desktop): deflake mid-scroll older-history spinner smoke test ([#1632](https://github.com/block/buzz/pull/1632)) ([`7ff58d9af`](https://github.com/block/buzz/commit/7ff58d9af9edbdfd3f1a8c508641dda12710320a))
- feat(desktop): add Buzz light + dark themes with branded sidebar gradient ([#1630](https://github.com/block/buzz/pull/1630)) ([`290be2a03`](https://github.com/block/buzz/commit/290be2a0395c1d2151ae5524c5474d95cffc302c))
- fix(desktop): render thread facepile oldest-replier-first ([#1595](https://github.com/block/buzz/pull/1595)) ([`f9701a386`](https://github.com/block/buzz/commit/f9701a38600302b3a97845b6f25681b6ef63dbd7))
- fix(desktop): keep sidebar section actions visible while ⋮ menu is open ([#1584](https://github.com/block/buzz/pull/1584)) ([`943d7a663`](https://github.com/block/buzz/commit/943d7a663028a32485f3a372a1ae73286c225e95))
- refactor(desktop): extract shared runtime/provider/model selection + env-var helpers (Phase 1B.1) ([#1624](https://github.com/block/buzz/pull/1624)) ([`4b5cac074`](https://github.com/block/buzz/commit/4b5cac074b10dab75404aa09f65d5a4e43ff4848))
- feat(desktop): fold personas.json into the unified agent store (Phase 1A.2) ([#1623](https://github.com/block/buzz/pull/1623)) ([`381c7714f`](https://github.com/block/buzz/commit/381c7714fa6943f033eb3963aa105397098b782b))
- fix(desktop): hash spawn config as the env receives it, not raw record fields ([#1621](https://github.com/block/buzz/pull/1621)) ([`f0eb4eccd`](https://github.com/block/buzz/commit/f0eb4eccd9dbae6819d44c282db675c89f8e972a))
- test(desktop): add edit-agent dialog e2e coverage (Phase 1B.3b-pre) ([#1628](https://github.com/block/buzz/pull/1628)) ([`a609e796b`](https://github.com/block/buzz/commit/a609e796bd1b5eda01c733041e5e5320c2f4557d))
- feat(desktop): unified AgentRecord groundwork — record-first resolution + runtime materialization (Phase 1A.1) ([#1618](https://github.com/block/buzz/pull/1618)) ([`5dce80aff`](https://github.com/block/buzz/commit/5dce80aff53ca698c1c17ac21866bc3949ed14bc))
- fix(desktop): backfill edits for thread replies so edited replies survive refetch ([#1610](https://github.com/block/buzz/pull/1610)) ([`d1f3194e7`](https://github.com/block/buzz/commit/d1f3194e73abd1389c62b32e0502fcf8c5ce9b4c))
- chore(desktop): clear desktop-tauri clippy backlog ([#1612](https://github.com/block/buzz/pull/1612)) ([`36fd41c5c`](https://github.com/block/buzz/commit/36fd41c5ce63bf0f8f0ad8d740fce47cae7ef9d1))
- fix(desktop): restrict shared-agent sync to dev data dirs ([#1597](https://github.com/block/buzz/pull/1597)) ([`e5f831d2c`](https://github.com/block/buzz/commit/e5f831d2c2358592e24a20fb020f43fc50ea9d1b))
- feat(desktop): restart-required badge from spawn-time config hash ([#1602](https://github.com/block/buzz/pull/1602)) ([`cc42a4979`](https://github.com/block/buzz/commit/cc42a497991ee6fe632734239c53a7c4d4fb4424))
- feat(desktop): boot-time reconcile of managed agents to relay events ([#1601](https://github.com/block/buzz/pull/1601)) ([`c5a541ee2`](https://github.com/block/buzz/commit/c5a541ee2d2436d0fb970e5c0164bed349a76c4a))
- feat(desktop): canonical <PubKey> component — hover to view/copy full keys, owner "you" labels ([#1589](https://github.com/block/buzz/pull/1589)) ([`777babf39`](https://github.com/block/buzz/commit/777babf3938a2ae7ef97fd12f12e6247e7c18cec))
- fix(desktop): hydrate reactions for Inbox context messages ([#1596](https://github.com/block/buzz/pull/1596)) ([`cdf982bdb`](https://github.com/block/buzz/commit/cdf982bdb3fa4693416104a3e26a324e7703b58c))


## v0.3.46

- fix(desktop): preserve agent model/provider when persona snapshot fields are blank ([#1583](https://github.com/block/buzz/pull/1583)) ([`a3ee2c569`](https://github.com/block/buzz/commit/a3ee2c5693d58f32b17072a49add2dbaff8e0ffd))
- feat(acp,desktop): identify and reap stale agent harness processes ([#1582](https://github.com/block/buzz/pull/1582)) ([`3c6c0d447`](https://github.com/block/buzz/commit/3c6c0d4478e5bb6e734cc16eafa6de6a433bf8fe))
- feat(desktop): active-draft badge, send-from-drafts confirm dialog, thread-deleted state ([#1581](https://github.com/block/buzz/pull/1581)) ([`b394e95ef`](https://github.com/block/buzz/commit/b394e95ef03b131fb43bda0954d900ae56473f31))
- fix(desktop): treat baked build env vars as satisfying required agent config ([#1580](https://github.com/block/buzz/pull/1580)) ([`b90e80443`](https://github.com/block/buzz/commit/b90e80443ea498e2b7b97bde9069d4b7c6ab4048))
- feat(desktop): add "Copy image" to image right-click context menu ([#1579](https://github.com/block/buzz/pull/1579)) ([`683f7fec1`](https://github.com/block/buzz/commit/683f7fec1c357f1140a4f874720f513b69d919a4))
- fix(nest): use buzz-dev symlink name for dev builds ([#1587](https://github.com/block/buzz/pull/1587)) ([`dcbb3ff78`](https://github.com/block/buzz/commit/dcbb3ff7892891cdc3c447049444cd2be8570f5e))
- fix(composer): address image-editor follow-up nits on #1491 ([#1565](https://github.com/block/buzz/pull/1565)) ([`49391d807`](https://github.com/block/buzz/commit/49391d80713b74d1f48a442b193d9a91b086d99a))
- fix(desktop): render black static boot screen ([#1570](https://github.com/block/buzz/pull/1570)) ([`10444d248`](https://github.com/block/buzz/commit/10444d248e26f6ab119cb13ccef906f42dc333da))
- feat(agents): group activity tool bursts ([#1571](https://github.com/block/buzz/pull/1571)) ([`426f04484`](https://github.com/block/buzz/commit/426f044844951fc1ced7670d2dd5d25812deced9))
- feat(desktop): aggregated overview rail, commit detail page, and full breadcrumbs ([#1573](https://github.com/block/buzz/pull/1573)) ([`c97263901`](https://github.com/block/buzz/commit/c97263901f9757ed6e9ddbf3f47097e2c6567e24))
- fix(desktop): fetch profiles for reaction actors and thread-reply authors ([#1550](https://github.com/block/buzz/pull/1550)) ([`62def1459`](https://github.com/block/buzz/commit/62def1459c6e4d2beb5fda9e0c4accde16aa2c8d))
- refactor(desktop): unify EditAgentDialog styling with PersonaDialog ([#1540](https://github.com/block/buzz/pull/1540)) ([`c258ffc4a`](https://github.com/block/buzz/commit/c258ffc4a87fb523900801d0959334912f25d30a))
- feat(desktop): add 10-minute message grouping window ([#1578](https://github.com/block/buzz/pull/1578)) ([`564ead385`](https://github.com/block/buzz/commit/564ead3856c1e9f5c8760aaa6bcc9bd16cbbd9be))
- feat(desktop): unify sidebar section actions into a per-section ⋮ menu ([#1577](https://github.com/block/buzz/pull/1577)) ([`d94561795`](https://github.com/block/buzz/commit/d94561795a48af743278bfa1e912b894228312f3))
- feat(desktop): emoji avatar picker for agents + reliable picker scroll ([#1576](https://github.com/block/buzz/pull/1576)) ([`ed84a8735`](https://github.com/block/buzz/commit/ed84a87358495a4cdd6e61ee1cfe01db9a367b35))
- fix(desktop): sync derived active-turn liveness in channel activity path ([#1492](https://github.com/block/buzz/pull/1492)) ([`1f5ba5bb2`](https://github.com/block/buzz/commit/1f5ba5bb27a94d33cce5aaa7cfea1d116c7d582d))
- fix(personas): remove goose runtime pin from Fizz built-in ([#1566](https://github.com/block/buzz/pull/1566)) ([`e3008b360`](https://github.com/block/buzz/commit/e3008b3604330803975d66920204e7365fc571a9))
- feat(acp,buzz-agent): thread model name through NIP-AM kind 44200 emit path ([#1564](https://github.com/block/buzz/pull/1564)) ([`08e707eb5`](https://github.com/block/buzz/commit/08e707eb573a187432a950154a15f9cb4785d25f))
- sync channel sort preferences across clients ([#1556](https://github.com/block/buzz/pull/1556)) ([`04a9d1029`](https://github.com/block/buzz/commit/04a9d10299b92c5b0e7597a53914b9a753985c51))
- fix desktop canvas header blur in split channel management ([#1558](https://github.com/block/buzz/pull/1558)) ([`2ef26ec21`](https://github.com/block/buzz/commit/2ef26ec21048857fc11982cd8c153e1c795f2c6d))
- feat(composer): compose spoiler, annotation, and tooltip controls ([#1491](https://github.com/block/buzz/pull/1491)) ([`1aa87bb26`](https://github.com/block/buzz/commit/1aa87bb26b96d13fa2bba042ccd933e988d79bae))
- feat(channel list): add persistent channel sort toggle ([#1505](https://github.com/block/buzz/pull/1505)) ([`36ac5243f`](https://github.com/block/buzz/commit/36ac5243ff8aed4401d3453a115afc4a603fccf0))


## v0.3.45

- chore(deps): bump crossbeam-epoch to 0.9.20 for RUSTSEC-2026-0204 ([#1563](https://github.com/block/buzz/pull/1563)) ([`75b52ad7e`](https://github.com/block/buzz/commit/75b52ad7e54ca7f930656e756910a08d0f146b82))
- fix(archive): atomic remove-kind path + split test modules ([#1562](https://github.com/block/buzz/pull/1562)) ([`484bede75`](https://github.com/block/buzz/commit/484bede7597814dd7dc322d1f72f69c08545ef42))
- feat(archive): add agent turn-metric (kind 44200) local archive ([#1555](https://github.com/block/buzz/pull/1555)) ([`429f54b4f`](https://github.com/block/buzz/commit/429f54b4f45df0d5f4a52aafb8913e903ebdc6b3))
- feat(desktop,buzz-acp): add harness-agnostic config bridge and setup-listener mode ([#1411](https://github.com/block/buzz/pull/1411)) ([`d8355d318`](https://github.com/block/buzz/commit/d8355d318853f85ffaf53dc7b5215bc998b460c4))
- fix(desktop): stop edit-channel dialog hanging on "Saving..." ([#1557](https://github.com/block/buzz/pull/1557)) ([`76806cb1f`](https://github.com/block/buzz/commit/76806cb1f654650a412b7f0368f495062a9ee224))
- feat(archive): add local-save archive with observer-feed default-on option ([#1442](https://github.com/block/buzz/pull/1442)) ([`711078807`](https://github.com/block/buzz/commit/711078807777db77960404e51ffd862c08a33ded))
- feat(desktop): add Slack-like Drafts inbox with persistence and image fix ([#1539](https://github.com/block/buzz/pull/1539)) ([`4c598dcef`](https://github.com/block/buzz/commit/4c598dcef577bb204f9666eaac5b95743a0f8736))
- fix(setup): fix syntax error in seed-local-community.sh ([#1547](https://github.com/block/buzz/pull/1547)) ([`3729a6515`](https://github.com/block/buzz/commit/3729a6515617abdae03117d9713dfeb7e1ff74c2))
- feat(desktop): add right-click context menu to workspace rail ([#1552](https://github.com/block/buzz/pull/1552)) ([`0e87998f8`](https://github.com/block/buzz/commit/0e87998f8fcac0ff1612d661cbd505a63b51c550))
- fix(desktop): restore saved window geometry on launch ([#1554](https://github.com/block/buzz/pull/1554)) ([`e790c9828`](https://github.com/block/buzz/commit/e790c9828f1ae85ce7618d06e67e78732be4221b))
- feat(desktop): add typeahead search to persona model dropdown ([#1542](https://github.com/block/buzz/pull/1542)) ([`3b43743e1`](https://github.com/block/buzz/commit/3b43743e1f2acd192705a80629ceadfa7a737831))
- fix(desktop): publish new agent profiles to active relay ([#1546](https://github.com/block/buzz/pull/1546)) ([`346efe08b`](https://github.com/block/buzz/commit/346efe08b3ca6e847b6c9c2f843cdb8834db123c))
- chore(desktop): use dot access for host header assignment ([#1548](https://github.com/block/buzz/pull/1548)) ([`ccfdf45bc`](https://github.com/block/buzz/commit/ccfdf45bc5dd61ef7d59644293d84deed24e1789))
- fix(onboarding): guard against webkit2gtk WAL race with explicit profile-event signal ([#1508](https://github.com/block/buzz/pull/1508)) ([`c2ee4d162`](https://github.com/block/buzz/commit/c2ee4d162753a3338dc434f85c80a82eb284b3d2))


## v0.3.44

- fix(desktop): keep thread-summary badges mounted through scrollback prepends ([#1533](https://github.com/block/buzz/pull/1533)) ([`6abf614fd`](https://github.com/block/buzz/commit/6abf614fd4eb512cf2e4175f17dc09892a8b472d))
- refactor(shell): drop bundled PortableGit, add BUZZ_SHELL override + dialect hint ([#1536](https://github.com/block/buzz/pull/1536)) ([`cfa208983`](https://github.com/block/buzz/commit/cfa20898313080b7a106ae0e092d8ed6815daeb0))
- feat(nips,relay,acp): NIP-AM durable encrypted agent turn metrics (kind 44200) ([#1441](https://github.com/block/buzz/pull/1441)) ([`71265ca36`](https://github.com/block/buzz/commit/71265ca36105dbf62453a99c998c3f3dd134a304))


## v0.3.43

- fix(desktop): sync agent relay profile when persona avatar changes ([#1512](https://github.com/block/buzz/pull/1512)) ([`ce901f7c1`](https://github.com/block/buzz/commit/ce901f7c1e548a0aab0d9e97f4951ea134f6b3d2))
- feat(desktop): redesign appearance settings with mode-first theme picker ([#1528](https://github.com/block/buzz/pull/1528)) ([`9e773f103`](https://github.com/block/buzz/commit/9e773f103a2829d1b0195a2bf4f01a4b3ff69291))
- fix(deps): restore windows-* crates downgraded by chrono bump (fixes Windows Rust on main) ([#1532](https://github.com/block/buzz/pull/1532)) ([`d40837085`](https://github.com/block/buzz/commit/d408370851643fca1525cdb00147e589e7408f21))
- chore(renovate): stop rebase churn between weekly sweeps ([#1530](https://github.com/block/buzz/pull/1530)) ([`a8238cd67`](https://github.com/block/buzz/commit/a8238cd670031bf3595fa8d0f2e79e6f64571054))
- fix(deps): unversion isomorphic-git pnpm patch key ([#1529](https://github.com/block/buzz/pull/1529)) ([`2a24765fd`](https://github.com/block/buzz/commit/2a24765fd74f66764890746a84f9654e578986f3))
- chore(deps): update dependency @tanstack/react-virtual to v3.14.5 ([#1523](https://github.com/block/buzz/pull/1523)) ([`b66a53f5b`](https://github.com/block/buzz/commit/b66a53f5b5ba4355511043ab621f1219417b0a4b))
- chore(deps): update redis to v1.2.2 ([#1049](https://github.com/block/buzz/pull/1049)) ([`f1b29140c`](https://github.com/block/buzz/commit/f1b29140c66e2987fedb0fa66174552754721d97))
- feat(desktop): restore section icon/emoji picker ([#1516](https://github.com/block/buzz/pull/1516)) ([`232b6066c`](https://github.com/block/buzz/commit/232b6066c0fe54e849537aaa8da644d42bc3e33c))
- chore(deps): update radix-ui-primitives monorepo ([#1524](https://github.com/block/buzz/pull/1524)) ([`dc383c199`](https://github.com/block/buzz/commit/dc383c1993c5dfe8fea95f4e10f1e5ee5648f7aa))
- fix(desktop): stop Leave-channel dialog from freezing the app ([#1482](https://github.com/block/buzz/pull/1482)) ([`668d30b0c`](https://github.com/block/buzz/commit/668d30b0cf7a466480d07d024583963dc51e2d51))
- perf: lazy-load avatars and message images ([#1517](https://github.com/block/buzz/pull/1517)) ([`d09d7dea5`](https://github.com/block/buzz/commit/d09d7dea558bcd17d66315ca099a5a2752a3be60))
- Live thread-summary push: badge counts update on reply ingest ([#1521](https://github.com/block/buzz/pull/1521)) ([`74a30c3de`](https://github.com/block/buzz/commit/74a30c3de8ff61005fecac53eb007e8a616a9229))
- chore(deps): update all non-major dependencies ([#1163](https://github.com/block/buzz/pull/1163)) ([`a255a7735`](https://github.com/block/buzz/commit/a255a7735855048065b3cc4e530b4af5a70f444d))
- chore(deps): update rust crate anyhow to v1.0.103 ([#1525](https://github.com/block/buzz/pull/1525)) ([`26d609cbe`](https://github.com/block/buzz/commit/26d609cbe1770b3bb5809c57bb0d66e775e6ac63))
- chore(deps): update cashapp/activate-hermit digest to cea9af7 ([#1522](https://github.com/block/buzz/pull/1522)) ([`19d98319a`](https://github.com/block/buzz/commit/19d98319a811cbc63dd34300ac732fa9941f054a))
- chore(deps): update rust crate chrono to v0.4.45 ([#1050](https://github.com/block/buzz/pull/1050)) ([`3caa66d4f`](https://github.com/block/buzz/commit/3caa66d4f047fced63626667261372d49d3c04a9))
- chore(deps): update actions/cache digest to caa2961 ([#1341](https://github.com/block/buzz/pull/1341)) ([`8dea817fa`](https://github.com/block/buzz/commit/8dea817fa97b384b7ea8a2f20789300d5e0a8c60))
- chore(deps): pin dependencies ([#1162](https://github.com/block/buzz/pull/1162)) ([`4b01040af`](https://github.com/block/buzz/commit/4b01040afb3cc778f3a2a1d8f89af37352b2ef35))
- feat(desktop): replace placeholder icon with actual app icon on welcome screen ([#1527](https://github.com/block/buzz/pull/1527)) ([`5b88e5956`](https://github.com/block/buzz/commit/5b88e595657cf4ffc740e30b187439fe53547331))
- feat(desktop): enable native spellcheck in message composer ([#1515](https://github.com/block/buzz/pull/1515)) ([`a1b5983e5`](https://github.com/block/buzz/commit/a1b5983e5400a6bd9f4c8056086de616931e33b4))
- fix: let agent owners delete their agent's messages (relay kind:5 + desktop/mobile UX) ([#1519](https://github.com/block/buzz/pull/1519)) ([`642800548`](https://github.com/block/buzz/commit/6428005487f0690019dc27449a6a52cc29cc6479))
- chore(desktop): bring ChannelScreen back under the size gate ([#1520](https://github.com/block/buzz/pull/1520)) ([`8174f2b0a`](https://github.com/block/buzz/commit/8174f2b0a0aee3d0252ef8923a043b2aa2773499))
- fix(zoom) desktop chrome clearance under text zoom ([#1490](https://github.com/block/buzz/pull/1490)) ([`ee4c9ef13`](https://github.com/block/buzz/commit/ee4c9ef139724a3cb82841d449674e6c96932b7b))
- fix(activity panel): handle back navigation ([#1487](https://github.com/block/buzz/pull/1487)) ([`caf644eda`](https://github.com/block/buzz/commit/caf644eda730c8472183e6801d428f3fd8c05ef7))
- Port channel windows to mobile ([#1518](https://github.com/block/buzz/pull/1518)) ([`6716cd31a`](https://github.com/block/buzz/commit/6716cd31a5303f3964a7acac8159c365d28f55f0))
- perf: GIN index for e-tag containment + delta profile fetch (scroll-back ~2.1s/page) ([#1514](https://github.com/block/buzz/pull/1514)) ([`33886e3de`](https://github.com/block/buzz/commit/33886e3dec4130e512d8242207adfe2811a92579))
- GUI read-model overhaul: server-assembled channel windows (Correct™ pagination + relay-signed bounds) ([#1500](https://github.com/block/buzz/pull/1500)) ([`62bb9fe8c`](https://github.com/block/buzz/commit/62bb9fe8c81eee6573c434ffa3227fa96ad9dd4b))
- feat(desktop): show activity timestamps on demand ([#1506](https://github.com/block/buzz/pull/1506)) ([`c7a8b0bab`](https://github.com/block/buzz/commit/c7a8b0babb35e296fbc4a1fc3df0015518085d32))
- feat(reconnect): replace top banner with animated sidebar overlay ([#1510](https://github.com/block/buzz/pull/1510)) ([`f35aeb798`](https://github.com/block/buzz/commit/f35aeb7986b2751f3ff2e4e33403aa5c28af0b43))
- docs(nest-skill): explain agent-owned git repos and automatic auth ([#1437](https://github.com/block/buzz/pull/1437)) ([`20d3cdc22`](https://github.com/block/buzz/commit/20d3cdc22f4a53a792c4b249ef0338ff6411581d))
- fix(agent): make stop-hook rejection budget per-prompt, fix stale hook docs ([#1503](https://github.com/block/buzz/pull/1503)) ([`4a0951042`](https://github.com/block/buzz/commit/4a095104297a58e50d3ffb0ce600162b9a298323))


## v0.3.42

- fix(desktop): bound read-state localStorage growth and recover from quota errors ([#1502](https://github.com/block/buzz/pull/1502)) ([`a3cf7eec1`](https://github.com/block/buzz/commit/a3cf7eec1d5c876c65aabc827620ca2b7b79127b))
- Customize macOS DMG installer ([#1496](https://github.com/block/buzz/pull/1496)) ([`bbf8d8912`](https://github.com/block/buzz/commit/bbf8d8912a19d9f8f249b3b7897999e343f803dd))
- mobile: thread scroll-to-bottom and desktop-parity mention autocomplete ([#1499](https://github.com/block/buzz/pull/1499)) ([`e9318f66d`](https://github.com/block/buzz/commit/e9318f66d8fe5bff1df8afb7a2dcc36baf0876ac))
- fix(agent): honor stop hook retry budget ([#1501](https://github.com/block/buzz/pull/1501)) ([`1c297d2f2`](https://github.com/block/buzz/commit/1c297d2f26688c35718e90a875e0bece41392253))
- feat(profile): embed live activity feed in profile aux panel ([#1380](https://github.com/block/buzz/pull/1380)) ([`654b6c374`](https://github.com/block/buzz/commit/654b6c374b3f8a2f2a59c72f604bc5de4546f53f))
- feat(desktop): contribution heatmap and graphical cards on projects overview ([#1497](https://github.com/block/buzz/pull/1497)) ([`204a0fd2e`](https://github.com/block/buzz/commit/204a0fd2ec2332cdbad7279be537aee27f20a70d))
- feat(desktop): repository-first projects with git workflows ([#1471](https://github.com/block/buzz/pull/1471)) ([`8e3c0ee95`](https://github.com/block/buzz/commit/8e3c0ee958af8777ba54fd835de03b0e8eada531))
- fix(desktop): lock horizontal webview pan (Magic Mouse side-scroll) ([#1480](https://github.com/block/buzz/pull/1480)) ([`5d4edf153`](https://github.com/block/buzz/commit/5d4edf1535c4b19543e1f57722f4dc94b5af675c))
- Add agent catalog modal ([#1302](https://github.com/block/buzz/pull/1302)) ([`228122fdd`](https://github.com/block/buzz/commit/228122fddcc35cef0cb69c89892276c9b5986c25))
- fix(desktop): actually close relay sockets — plugin:websocket|disconnect does not exist ([#1481](https://github.com/block/buzz/pull/1481)) ([`e70dd1a1e`](https://github.com/block/buzz/commit/e70dd1a1e70cc0f56aa6094e4da22cd91e3203b0))
- feat(buzz-agent): config parity — thinking effort, model switching, normalized token limits ([#1470](https://github.com/block/buzz/pull/1470)) ([`3e282a241`](https://github.com/block/buzz/commit/3e282a24181aeef1c2cf06d4e4bc3b1c87f60067))
- fix(sidebar): scope channel sections storage to relay URL ([#1477](https://github.com/block/buzz/pull/1477)) ([`f9d06ae21`](https://github.com/block/buzz/commit/f9d06ae21aaa20c3de299a6f7065863e9e8d76b3))
- chore(mobile): declare non-exempt encryption usage in Info.plist ([#1474](https://github.com/block/buzz/pull/1474)) ([`453b8b1e4`](https://github.com/block/buzz/commit/453b8b1e47d0a13fb568b2f9b797113c4d663c0f))
- fix(desktop): bind channel and thread context at compose time to prevent wrong-channel send ([#1472](https://github.com/block/buzz/pull/1472)) ([`d369ca9df`](https://github.com/block/buzz/commit/d369ca9df1248e2ee16a40b2a193bf08dd8126c4))
- fix(relay-reconnect): resilient reconnect with fast-path, escalation, and polling ([#1456](https://github.com/block/buzz/pull/1456)) ([`02ff06cac`](https://github.com/block/buzz/commit/02ff06cac230ae708e1dffaa10bf3f71351ae582))
- feat: per-community workspace icon set by admins, served via NIP-11 ([#1463](https://github.com/block/buzz/pull/1463)) ([`5bfd5ca27`](https://github.com/block/buzz/commit/5bfd5ca2700483498e83224a40a5628a29cf2e9e))
- perf(relay): batch outbound websocket data frames ([#1464](https://github.com/block/buzz/pull/1464)) ([`01b92faa1`](https://github.com/block/buzz/commit/01b92faa156648835f143e84583b8ec3bd7490ab))
- Make reaction ingest atomic ([#1458](https://github.com/block/buzz/pull/1458)) ([`835302cc8`](https://github.com/block/buzz/commit/835302cc829c8a63bf254d3e40156fc446e040f6))
- Serialize fan-out EVENT frames once ([#1459](https://github.com/block/buzz/pull/1459)) ([`3c661fb48`](https://github.com/block/buzz/commit/3c661fb48f81c294b529592d2b2ff874bf96ee96))
- fix: agent reliability — no restart on channel-add, visible dead-letter notice ([#1468](https://github.com/block/buzz/pull/1468)) ([`d9c4e4aa7`](https://github.com/block/buzz/commit/d9c4e4aa7fb5634f19d96f31da9f602951f503d0))
- fix(profile): consolidate agent profile runtime metadata ([#1451](https://github.com/block/buzz/pull/1451)) ([`c48006fc3`](https://github.com/block/buzz/commit/c48006fc3e2e3a2a7984be1f02f6e6ba9d11b8eb))
- fix(desktop): simplify workspace rail badges ([#1462](https://github.com/block/buzz/pull/1462)) ([`e42dae3f9`](https://github.com/block/buzz/commit/e42dae3f9ce767e832e1b954875e3bf3e662d35c))
- perf(desktop): instant channel switching — non-blocking first paint, persisted snapshots ([#1452](https://github.com/block/buzz/pull/1452)) ([`deb3e6adc`](https://github.com/block/buzz/commit/deb3e6adcaeb744439e794a71090d2d1dcfc004c))
- perf(relay): bounded-concurrency multi-filter query execution (S2) ([#1457](https://github.com/block/buzz/pull/1457)) ([`a9e752e25`](https://github.com/block/buzz/commit/a9e752e2540a94d304a51ddeecbf68464ca9ec69))
- fix(desktop): classify timeline prepends so history loads don't bump unread ([#1416](https://github.com/block/buzz/pull/1416)) ([`9967b97f5`](https://github.com/block/buzz/commit/9967b97f59179a0261ef5e2046df9632652be619))
- fix(desktop): quiet gate for workspace switches instead of boot splash ([#1449](https://github.com/block/buzz/pull/1449)) ([`b779a3ee2`](https://github.com/block/buzz/commit/b779a3ee2f4fea3599d61511ae82fed439c919ce))
- fix(read-path): reach complete threads, dense-second timelines, and all people in the GUI ([#1418](https://github.com/block/buzz/pull/1418)) ([`7da936fff`](https://github.com/block/buzz/commit/7da936fff82a9a956f338c690e9605888725ea3b))
- E1+E3: reduce relay ingest/fan-out DB round trips; ack p99 −7–16%, fd p99 −6–28%, p999 tails −29–53% vs PR #1453 tip ([#1454](https://github.com/block/buzz/pull/1454)) ([`a504ad619`](https://github.com/block/buzz/commit/a504ad6197558575c0db7b9f53806d7337e0c64f))
- perf(relay): defer post-commit dispatch and avoid verify clone ([#1453](https://github.com/block/buzz/pull/1453)) ([`7bd3760c8`](https://github.com/block/buzz/commit/7bd3760c82a6d640af199ed2301525877e629ced))
- fix(relay): include git hook tools in runtime image ([#1326](https://github.com/block/buzz/pull/1326)) ([`88c089d3b`](https://github.com/block/buzz/commit/88c089d3b652bc952adbe8b32a6fc585121c982f))
- feat(chart): per-pod emptyDir git scratch when persistence disabled (multi-replica HA) ([#1450](https://github.com/block/buzz/pull/1450)) ([`c88799ac6`](https://github.com/block/buzz/commit/c88799ac6c3b5b149196223abe7a6134c8823359))
- fix(relay): remove media bearer-token auth ([#1444](https://github.com/block/buzz/pull/1444)) ([`0701f47f4`](https://github.com/block/buzz/commit/0701f47f4a31a904ebcd9f360cbd6aadaff9d784))
- fix(desktop): stop search shortcut from hijacking the sidebar ([#1447](https://github.com/block/buzz/pull/1447)) ([`15ad7ae87`](https://github.com/block/buzz/commit/15ad7ae87e655a7873fa0b0a51f53b5f800afd9b))
- fix(ci): set PGSCHEMA_PLAN_* in start-relay-for-tests.sh to avoid embedded-PG fetch ([#1443](https://github.com/block/buzz/pull/1443)) ([`89c4f7657`](https://github.com/block/buzz/commit/89c4f76579944ae1c7f86e37d76cecf2b378e60e))
- feat(desktop): restore observer-feed regressions from #1381 and classify 4 new session/update types ([#1412](https://github.com/block/buzz/pull/1412)) ([`fec768436`](https://github.com/block/buzz/commit/fec76843665aeb4aff06688f83b93fc6ed2b603d))
- fix(desktop): disable spellcheck/autocorrect/autocapitalize on emoji picker search ([#1438](https://github.com/block/buzz/pull/1438)) ([`bdeab23b5`](https://github.com/block/buzz/commit/bdeab23b59ca8e622e5c23b3368ecaccf21c5652))
- feat(relay): add OpenTelemetry tracing, keep Prometheus metrics ([#1398](https://github.com/block/buzz/pull/1398)) ([`b1d9d955d`](https://github.com/block/buzz/commit/b1d9d955de83538c231a3034bf190af6df03070d))
- feat(buzz-agent): emit agent_thought_chunk for reasoning content ([#1436](https://github.com/block/buzz/pull/1436)) ([`9f2a11b33`](https://github.com/block/buzz/commit/9f2a11b33827036d7b7415ede72c1dacd0fcd6f9))
- feat(git): move repo-name registry to Postgres + relax RWM chart gate (HA relay) ([#1432](https://github.com/block/buzz/pull/1432)) ([`e5aa4a213`](https://github.com/block/buzz/commit/e5aa4a21327438c02fb25baea4d0849a498c9059))


## v0.3.41

- Group consecutive desktop messages ([#1429](https://github.com/block/buzz/pull/1429)) ([`3d08c3b0`](https://github.com/block/buzz/commit/3d08c3b02b284d062e2932df8f96e2467bb40946))
- Update Buzz app icon ([#1430](https://github.com/block/buzz/pull/1430)) ([`db2a9701`](https://github.com/block/buzz/commit/db2a97011536e810abdc3f813ec1867cbcef21f6))
- feat(desktop): add workspace rail unread observer ([#1428](https://github.com/block/buzz/pull/1428)) ([`ce1f13e8`](https://github.com/block/buzz/commit/ce1f13e8f7a36621621d9249f7535a33439cfb87))
- Prioritize channel members in mention autocomplete ([#1431](https://github.com/block/buzz/pull/1431)) ([`8fb33bdb`](https://github.com/block/buzz/commit/8fb33bdbca4285cac379ad18d42533859eacbd12))
- Tighten message density ([#1426](https://github.com/block/buzz/pull/1426)) ([`6a08a3f4`](https://github.com/block/buzz/commit/6a08a3f4c8f79c2ed4e9ff7e425f3faa0b3627e5))
- Tighten sidebar section actions ([#1424](https://github.com/block/buzz/pull/1424)) ([`c0e10d67`](https://github.com/block/buzz/commit/c0e10d67e06d405829ad8c4682b3cb621702d150))
- Add memory copy actions ([#1427](https://github.com/block/buzz/pull/1427)) ([`6c0e6f0b`](https://github.com/block/buzz/commit/6c0e6f0bb74fe0b4777afbdb8151dddf7107c99f))
- Limit agent profile quick actions ([#1425](https://github.com/block/buzz/pull/1425)) ([`177fe5a3`](https://github.com/block/buzz/commit/177fe5a3bfe7774205ca0a9de689924a5df988df))
- Fade sidebar pinned chrome edges ([#1423](https://github.com/block/buzz/pull/1423)) ([`3e4e9dda`](https://github.com/block/buzz/commit/3e4e9dda94e04a67b6b73c372e853e961cca461d))
- Apply smooth corners to inline media ([#1422](https://github.com/block/buzz/pull/1422)) ([`2fc8b9cf`](https://github.com/block/buzz/commit/2fc8b9cf5f14410750a97bbebc07241ce8628a1f))
- Align thread summary rows ([#1421](https://github.com/block/buzz/pull/1421)) ([`4ae5a0d5`](https://github.com/block/buzz/commit/4ae5a0d5e372c5617aa32242c8e08c13a03d20b5))
- Keep channel day dividers sticky ([#1420](https://github.com/block/buzz/pull/1420)) ([`8c388479`](https://github.com/block/buzz/commit/8c3884791361933f89e6771e1403e4799fc00c1d))
- fix(deps): pin aws-creds to fork with EKS Pod Identity support ([#1419](https://github.com/block/buzz/pull/1419)) ([`86d6388e`](https://github.com/block/buzz/commit/86d6388e68d40aaa5449e3021e116a57cb2aefe0))
- fix(relay): enable Redis TLS for rediss:// (ElastiCache) ([#1417](https://github.com/block/buzz/pull/1417)) ([`3292b502`](https://github.com/block/buzz/commit/3292b502aad44a5f849296d7bf28429bac272fb7))


## v0.3.40

- fix(desktop): stabilize channel-timeline scrollback with per-row height reserves ([#1413](https://github.com/block/buzz/pull/1413)) ([`4fdc68f1`](https://github.com/block/buzz/commit/4fdc68f1568364b3e44d7003c83a9a4ad961e1ee))
- fix(sidebar): trim working badge label and name working agents in tooltip ([#1408](https://github.com/block/buzz/pull/1408)) ([`697f63dd`](https://github.com/block/buzz/commit/697f63ddcc45df71e48a0c0ac81adada67a056e1))
- Mobile tab bar polish ([#1368](https://github.com/block/buzz/pull/1368)) ([`c444e344`](https://github.com/block/buzz/commit/c444e3445bba4967246f790a296237cc597336a9))
- feat(desktop): let thread pane expand on ultrawide monitors ([#1407](https://github.com/block/buzz/pull/1407)) ([`f86f97bb`](https://github.com/block/buzz/commit/f86f97bbde1e3ee914ef2d6ef385482f15a43d6a))


## v0.3.39

- fix: close cross-process keychain race and namespace dev-build nest ([#1409](https://github.com/block/buzz/pull/1409)) ([`e8adc3383`](https://github.com/block/buzz/commit/e8adc3383fae3060124ed212d00538d543be0054))
- feat(relay): allow agent owners to edit/manage agent-owned content ([#1403](https://github.com/block/buzz/pull/1403)) ([`0042c8e10`](https://github.com/block/buzz/commit/0042c8e106d952f408cf4afca052f7053a7c967e))
- fix(media): support IRSA/credential-chain S3 auth and configurable signing region ([#1406](https://github.com/block/buzz/pull/1406)) ([`06ef533ec`](https://github.com/block/buzz/commit/06ef533ec7fec6cf7366f52d3b9fe2f83011bf24))
- fix(desktop): fold baked build env into in-process model discovery ([#1376](https://github.com/block/buzz/pull/1376)) ([`f061ae9c8`](https://github.com/block/buzz/commit/f061ae9c879e95f650fb99347920763361d1fe22))
- docs: link VISION_ACTIVITY from the VISION index ([#1405](https://github.com/block/buzz/pull/1405)) ([`aa1042b16`](https://github.com/block/buzz/commit/aa1042b1629466a94983d098e706a3905c645e9d))
- test(desktop): gate keychain-write test behind --ignored ([#1404](https://github.com/block/buzz/pull/1404)) ([`36f32291e`](https://github.com/block/buzz/commit/36f32291e79fc2069b8ef4cae3c90915232f702a))
- Refactor oversized mobile widgets ([#1401](https://github.com/block/buzz/pull/1401)) ([`71b2c41e9`](https://github.com/block/buzz/commit/71b2c41e927ca9c4aa2fd0f0c4662d4c8de6479a))
- perf(desktop): stop beachballs on agents menu and thread open ([#1402](https://github.com/block/buzz/pull/1402)) ([`34d61acc9`](https://github.com/block/buzz/commit/34d61acc9545676bb33e1675de2ab9f226242a29))
- feat(desktop): rebuild agent activity feed around a classifier registry and twelve render classes ([#1381](https://github.com/block/buzz/pull/1381)) ([`5cc09e698`](https://github.com/block/buzz/commit/5cc09e6989198df3918db8bc67e3bf020e6e0838))
- fix(thread): stop mid-scroll content jump in live threads ([#1397](https://github.com/block/buzz/pull/1397)) ([`42ec17f13`](https://github.com/block/buzz/commit/42ec17f1375f8886cda817921addda2c3bf55fd9))
- fix(ci): restore main to green — tauri fmt, personas.rs file-size split, Windows path test ([#1399](https://github.com/block/buzz/pull/1399)) ([`67c47de69`](https://github.com/block/buzz/commit/67c47de69c8a0028bcc35c1f5e9de489a8df9fb5))
- fix(desktop): enable buzz-dev-mcp MCP server for Codex agents ([#1394](https://github.com/block/buzz/pull/1394)) ([`b74ed858f`](https://github.com/block/buzz/commit/b74ed858ff337b9db186215b9cbbe43c89d63132))
- fix(ci): restore E2E flakiness fixes for pgschema, docker-pull, and spec timing ([#1396](https://github.com/block/buzz/pull/1396)) ([`fdf29c457`](https://github.com/block/buzz/commit/fdf29c457d0a4b10f99a703130786f71f724aa69))
- fix(personas): persist pack-backed persona UI edits across reboot ([#1392](https://github.com/block/buzz/pull/1392)) ([`a7e1202cc`](https://github.com/block/buzz/commit/a7e1202cc545983d2fc1449dcca2d778d5c0f88d))
- fix(buzz-acp): clear steer_rx on all run_prompt_task exit paths ([#1391](https://github.com/block/buzz/pull/1391)) ([`10aaa72f0`](https://github.com/block/buzz/commit/10aaa72f017b2d4cb1423de10c36718097fcb6c0))
- Restore channel date divider rule ([#1395](https://github.com/block/buzz/pull/1395)) ([`0827171d5`](https://github.com/block/buzz/commit/0827171d5e1db6332bfb819fcf4d8441ac82be1e))
- Speed up profile wave action ([#1379](https://github.com/block/buzz/pull/1379)) ([`2fc5dd35b`](https://github.com/block/buzz/commit/2fc5dd35b1cf48297613f7c7caba811f5248b431))
- Restore visible links for rich previews ([#1378](https://github.com/block/buzz/pull/1378)) ([`8d8fc6331`](https://github.com/block/buzz/commit/8d8fc6331f11da2904c2798c56c7833c13dfa090))
- Mobile channel list polish ([#1367](https://github.com/block/buzz/pull/1367)) ([`09d8965ab`](https://github.com/block/buzz/commit/09d8965ab686e5c910b7a04a20df58488f50a60c))
- style(desktop): unify corner radii to rounded-2xl (16px) ([#1393](https://github.com/block/buzz/pull/1393)) ([`2f496debc`](https://github.com/block/buzz/commit/2f496debce4961c88a20d7c0b497ec99626413b2))
- fix(desktop): skip keychain write when blob contents are unchanged ([#1377](https://github.com/block/buzz/pull/1377)) ([`5a64ee4a6`](https://github.com/block/buzz/commit/5a64ee4a69d46d9cef816a04c14b9abc9313eaa6))
- fix(desktop): stop clipping the agent-activity row under the composer ([#1371](https://github.com/block/buzz/pull/1371)) ([`2044ad76f`](https://github.com/block/buzz/commit/2044ad76f3dfd9f260a48cef176bed3b054f637d))
- Constrain macOS overscroll to conversations ([#1317](https://github.com/block/buzz/pull/1317)) ([`9b711f47a`](https://github.com/block/buzz/commit/9b711f47a791810a7c46069648d1a280b9739c6e))
- Mobile appearance foundation ([#1366](https://github.com/block/buzz/pull/1366)) ([`945e9b879`](https://github.com/block/buzz/commit/945e9b879ae2935312698a1e913cfb3009f8c10d))


## v0.3.38

- feat(desktop): provider-agnostic model selection + databricks discovery ([#1307](https://github.com/block/buzz/pull/1307)) ([`eacbbe880`](https://github.com/block/buzz/commit/eacbbe880a50acf400ff7c162b5bc8705ab0063f))
- release(helm): buzz chart 0.1.1 ([#1374](https://github.com/block/buzz/pull/1374)) ([`2561cbd06`](https://github.com/block/buzz/commit/2561cbd069a4f7a0ca4824f780867aa30ea9f744))
- Harden relay attack surfaces ([#1369](https://github.com/block/buzz/pull/1369)) ([`29368cf17`](https://github.com/block/buzz/commit/29368cf17b7d5924fe571512b2194e3f48b21a16))
- ci(helm): publish chart to GHCR on chart-v* tags ([#1372](https://github.com/block/buzz/pull/1372)) ([`2722ce422`](https://github.com/block/buzz/commit/2722ce4226838272ab36dc8630feacd2a90e1775))
- feat(buzz-agent): add databricks_v2 provider for AI Gateway v2 ([#1311](https://github.com/block/buzz/pull/1311)) ([`15a73aa27`](https://github.com/block/buzz/commit/15a73aa27feb9db72c9535a8b1113189d2be8dd4))
- refactor(desktop): centralize auxiliary panel shell ([#1343](https://github.com/block/buzz/pull/1343)) ([`e6738c501`](https://github.com/block/buzz/commit/e6738c50153cdb3f78448c912a2e8cd660da5779))
- perf(desktop): stop typing from re-rendering the channel pane + timeline ([#1364](https://github.com/block/buzz/pull/1364)) ([`db1b617ab`](https://github.com/block/buzz/commit/db1b617ab12792b6c5d90d988e52b9a86d7aa361))
- fix(desktop): check PGID in orphan sweep and signal correct process groups ([#1359](https://github.com/block/buzz/pull/1359)) ([`59be27ff3`](https://github.com/block/buzz/commit/59be27ff3fa4c0b0b95688565e945f6f5e0d60ef))
- feat(desktop): harness-agnostic config bridge ([#887](https://github.com/block/buzz/pull/887)) ([`c65989a61`](https://github.com/block/buzz/commit/c65989a61bba7050e4fbc1798630f96d541ab8db))
- fix(acp): enable sandbox network access for Codex MCP subprocesses ([#1363](https://github.com/block/buzz/pull/1363)) ([`401bb51bf`](https://github.com/block/buzz/commit/401bb51bf2dd64776a1862575548cb25ab41ff70))
- perf(desktop): don't block channel create on channel-list refetch ([#1360](https://github.com/block/buzz/pull/1360)) ([`cf57bcbea`](https://github.com/block/buzz/commit/cf57bcbea4af355774d499e109238a5d527ae39e))
- refactor(desktop): split global styles and markdown renderers ([#1361](https://github.com/block/buzz/pull/1361)) ([`c27b4251a`](https://github.com/block/buzz/commit/c27b4251a9304911f2359a3ee22b6f26b13af78b))
- feat(read-state): multi-slot splitting + no-op suppression for oversized blobs ([#1309](https://github.com/block/buzz/pull/1309)) ([`2612324fd`](https://github.com/block/buzz/commit/2612324fd935b3f19ef78e07885aa05f7ef907f1))
- fix: unconditionally replace CLI symlink on boot ([#1357](https://github.com/block/buzz/pull/1357)) ([`50873ef98`](https://github.com/block/buzz/commit/50873ef98d9080614720cd770781698c9b16313d))


## v0.3.37

- feat(buzz-acp): steering as the default mid-turn mention delivery ([#1160](https://github.com/block/buzz/pull/1160)) ([`e567491a`](https://github.com/block/buzz/commit/e567491a196396658ef4c1d4ff6128efd8b2744e))
- Multi-tenant Buzz relay: community_id as a server-resolved key (comprehensive rewrite) ([#1321](https://github.com/block/buzz/pull/1321)) ([`14fba21e`](https://github.com/block/buzz/commit/14fba21e57b8d671ebbea473226be52a5f2ae636))
- Disable persona start while runtime discovery runs ([#1353](https://github.com/block/buzz/pull/1353)) ([`92a5f1fc`](https://github.com/block/buzz/commit/92a5f1fc6624066f391bd5c57e9bb433df613b56))
- chore(deps): update dependency @tanstack/react-virtual to v3.14.4 ([#1342](https://github.com/block/buzz/pull/1342)) ([`1c221eaa`](https://github.com/block/buzz/commit/1c221eaafd1c8537532c3c8ce76c502330b00bf7))
- Fix sidebar unread indicator placement ([#1319](https://github.com/block/buzz/pull/1319)) ([`51b2613c`](https://github.com/block/buzz/commit/51b2613c0e42327d9ba0352c3bb3be7e7c4737a6))
- fix(desktop): un-clip hover action bar's upward bleed under content-visibility ([#1354](https://github.com/block/buzz/pull/1354)) ([`7adbd05b`](https://github.com/block/buzz/commit/7adbd05b45db3db0abb7cc1f5eb0dc7248d47711))
- Allow Huddle between 2 humans in DM ([#1347](https://github.com/block/buzz/pull/1347)) ([`b58f671b`](https://github.com/block/buzz/commit/b58f671b4ee3d9f6a8fd51ca23f0dc8b8036ed36))


## v0.3.36

- Polish agent runtime cards ([#1327](https://github.com/block/buzz/pull/1327)) ([`a3c4f3f6`](https://github.com/block/buzz/commit/a3c4f3f625a23addb6053c12f11f79f583c394c1))
- Rework desktop message-timeline scrolling: de-virtualize + native overflow-anchor ([#1338](https://github.com/block/buzz/pull/1338)) ([`4d619693`](https://github.com/block/buzz/commit/4d6196934e1f082ff5f16277edd33e3108aee38f))
- Keep wave huddles pending for placeholder profiles ([#1349](https://github.com/block/buzz/pull/1349)) ([`c1d6f3f2`](https://github.com/block/buzz/commit/c1d6f3f291b0f419bf0666f258d4689041747327))
- Polish profile hover cards ([#1346](https://github.com/block/buzz/pull/1346)) ([`e3fc0e02`](https://github.com/block/buzz/commit/e3fc0e0278d76b545adb6e7921387da9eeb7fdc3))
- Fix channel header shared blur layering ([#1336](https://github.com/block/buzz/pull/1336)) ([`360d2e54`](https://github.com/block/buzz/commit/360d2e54409eec21ec59a31c9a12d9d7821506a3))
- Add rich link previews ([#1334](https://github.com/block/buzz/pull/1334)) ([`bc925780`](https://github.com/block/buzz/commit/bc925780eed2169dd6dba9a2c9f85d05e2c9de1b))
- Revamp new agent dialog ([#1201](https://github.com/block/buzz/pull/1201)) ([`826d735f`](https://github.com/block/buzz/commit/826d735fe6712be820616d4cb1d6228cfe3be47e))
- Image attachment gallery lightbox ([#1345](https://github.com/block/buzz/pull/1345)) ([`7d3ee683`](https://github.com/block/buzz/commit/7d3ee6833665a4370db51b1ce0fa13f7174d9279))
- Reset prevent-sleep cap on agent activity ([#1335](https://github.com/block/buzz/pull/1335)) ([`433b1794`](https://github.com/block/buzz/commit/433b1794d55b568f8a1331d18536d136a1a03463))
- docs: update sprout repository references and document buzz mem ([#1333](https://github.com/block/buzz/pull/1333)) ([`52b6365e`](https://github.com/block/buzz/commit/52b6365e17529f385864433210985813023476ff))
- fix(buzz-agent): charge images a token-equivalent for the handoff gate ([#1332](https://github.com/block/buzz/pull/1332)) ([`744c77bc`](https://github.com/block/buzz/commit/744c77bc2d2db7d57a07a7701042db73ca40faa2))
- Polish thread reply hover states ([#1329](https://github.com/block/buzz/pull/1329)) ([`aca40b62`](https://github.com/block/buzz/commit/aca40b62899248359c2587efaae80c44009b87bb))
- Add persona and team import polish ([#1203](https://github.com/block/buzz/pull/1203)) ([`f00c86e7`](https://github.com/block/buzz/commit/f00c86e7446b6addfc83870f7d0571373642afd4))
- fix(desktop): reserve PTT shortcut only during active huddle ([#1315](https://github.com/block/buzz/pull/1315)) ([`6c60cb59`](https://github.com/block/buzz/commit/6c60cb59abb3e086281dd470390ba7e87bc5ab25))


## v0.3.35

- fix(desktop): split lib modules under size guard ([#1314](https://github.com/block/buzz/pull/1314)) ([`e7d43dc2`](https://github.com/block/buzz/commit/e7d43dc2253f0d1efe7689ac82a8b6b4a7788fd0))
- Fix desktop notifications on GNOME 46+ Linux ([#1246](https://github.com/block/buzz/pull/1246)) ([`ca50d832`](https://github.com/block/buzz/commit/ca50d832892f6203a59eeaadf3fe7b7e8b8e9888))
- perf(desktop): debounce channel-list refetch + profile get_channels ([#1310](https://github.com/block/buzz/pull/1310)) ([`c6e3e947`](https://github.com/block/buzz/commit/c6e3e947abc2592eea08c7601adc76afe0c14c95))
- feat(desktop): move agent management into profile sidebar ([#1274](https://github.com/block/buzz/pull/1274)) ([`8d40150c`](https://github.com/block/buzz/commit/8d40150c8377df08dbaed61de987ca69d45d15e0))
- feat(desktop): re-land virtualized timeline to fix macOS beachball ([#1250](https://github.com/block/buzz/pull/1250)) ([`8c3d0c92`](https://github.com/block/buzz/commit/8c3d0c92e83f1b482dbd93f2bd0d307988c91788))
- feat(acp): add BUZZ_ACP_ALLOWED_RESPOND_TO and BUZZ_ALLOWED_CHANNEL_ADD_POLICIES gates ([#1304](https://github.com/block/buzz/pull/1304)) ([`1a61d783`](https://github.com/block/buzz/commit/1a61d783ad072eefd03bb070be66ec2cf889dbde))
- fix(read-state): enforce byte-budget eviction in currentContexts() ([#1305](https://github.com/block/buzz/pull/1305)) ([`6b056461`](https://github.com/block/buzz/commit/6b0564618bafd3737778511b291e3b80ab7fc43e))
- feat(media): transcode HEIC/HEIF to JPEG on desktop upload ([#1257](https://github.com/block/buzz/pull/1257)) ([`d32f3c0a`](https://github.com/block/buzz/commit/d32f3c0a58dc3a3ce52b72cc0a8899228f17d69e))
- Bring mobile sidebar unread and DM parity ([#1303](https://github.com/block/buzz/pull/1303)) ([`1843a057`](https://github.com/block/buzz/commit/1843a057477129e392518dcc62a9814717a67c0b))
- Multi-tenant relay: spec + mechanized formal proof (S1–S8) ([#1285](https://github.com/block/buzz/pull/1285)) ([`2ecdcce7`](https://github.com/block/buzz/commit/2ecdcce7bdb5471cddf79cb5d4ce486a75ed2fda))
- Polish side panel motion and composer alignment ([#1294](https://github.com/block/buzz/pull/1294)) ([`34b2d3e8`](https://github.com/block/buzz/commit/34b2d3e8b43af132111e878288f1c4b4aa7777dd))
- feat(mobile): harden unread badges and float tabs ([#1298](https://github.com/block/buzz/pull/1298)) ([`59b21592`](https://github.com/block/buzz/commit/59b21592c92d55f6bd13524697f9cef257d9e4fe))
- feat(desktop): restore archive identity UI in profile panel ([#961](https://github.com/block/buzz/pull/961)) ([`4fce5aab`](https://github.com/block/buzz/commit/4fce5aab2e09a16a954cf6b685b7e2c1e897bd27))
- fix(sidebar): non-selectable channel names + copy/leave context menu actions ([#1260](https://github.com/block/buzz/pull/1260)) ([`4481f8fd`](https://github.com/block/buzz/commit/4481f8fd5a9cd8cd5a482a78ed63e2a34e600066))
- fix(runtime): sweep node wrapper processes hosting managed agent shims ([#1296](https://github.com/block/buzz/pull/1296)) ([`f072032a`](https://github.com/block/buzz/commit/f072032aaf52e216eecf652cd8625367c8c943ce))
- fix(buzz-agent): follow symlinks when discovering skill directories ([#1295](https://github.com/block/buzz/pull/1295)) ([`8717ddf2`](https://github.com/block/buzz/commit/8717ddf2ebc8cc6324996f9e5c16b596d1febc71))
- chore: add grab-emoji.sh to register Slack emoji in Buzz ([#1292](https://github.com/block/buzz/pull/1292)) ([`dfa864fc`](https://github.com/block/buzz/commit/dfa864fc457baa66dc9f4b4f501cccdd0d219e55))
- Fix cross-pod membership notification fanout ([#1291](https://github.com/block/buzz/pull/1291)) ([`e1c51d71`](https://github.com/block/buzz/commit/e1c51d71d48f9ad0a08c3e7e0af35929a0234ea1))
- fix(buzz-acp): strengthen agent communication rules in base prompt ([#1293](https://github.com/block/buzz/pull/1293)) ([`ccdb8975`](https://github.com/block/buzz/commit/ccdb8975d3683cd4710b52443284025aa6d05cd9))


## v0.3.34

- feat(desktop): refresh Agents tab live on inbound relay sync ([#1256](https://github.com/block/buzz/pull/1256)) ([`9a59e308`](https://github.com/block/buzz/commit/9a59e30817c642b1e6aab296863570869e215a19))
- fix(buzz-acp): inject Codex network allowlist for relay hostname at spawn time ([#1287](https://github.com/block/buzz/pull/1287)) ([`0379247e`](https://github.com/block/buzz/commit/0379247eba8a6c7e48a94f3fb8a329e7bdbca13c))
- refactor(desktop): consolidate notification helpers and add channel names to toasts ([#1286](https://github.com/block/buzz/pull/1286)) ([`dd5592e3`](https://github.com/block/buzz/commit/dd5592e34fcd7690340633dcec368db485557294))
- feat(buzz-agent): lazy skill loading via load_skill tool ([#1283](https://github.com/block/buzz/pull/1283)) ([`25396c06`](https://github.com/block/buzz/commit/25396c06d65c5e9a6d1271559cde4230e06ff35d))
- fix(buzz-acp): human-aware reply anchoring to keep threads flat ([#1281](https://github.com/block/buzz/pull/1281)) ([`6c920d21`](https://github.com/block/buzz/commit/6c920d21a852d301effb8a667d824e6b96485a7d))
- feat(desktop): add 'Follow system' theme mode ([#1262](https://github.com/block/buzz/pull/1262)) ([`c996be39`](https://github.com/block/buzz/commit/c996be395e23e50e5097f088738b3de9176759d4))
- Fix mention autocomplete layout in narrow threads ([#1282](https://github.com/block/buzz/pull/1282)) ([`227518c2`](https://github.com/block/buzz/commit/227518c234175440f9cd3734d01b96bbbbc89065))
- Clean up screenshot e2e tests ([#1284](https://github.com/block/buzz/pull/1284)) ([`2499d339`](https://github.com/block/buzz/commit/2499d33971524774f07f5186dadc8b33e055d812))
- fix(desktop): DM close button replaces unread badge on hover ([#1280](https://github.com/block/buzz/pull/1280)) ([`e3663721`](https://github.com/block/buzz/commit/e3663721830d79df6562a0b3ef78c8fe6bc1b7c3))
- Add NIP-34 git pull request CLI support ([#1279](https://github.com/block/buzz/pull/1279)) ([`5fef9b72`](https://github.com/block/buzz/commit/5fef9b727fb8856d45840ccc97d8ec820a48f6e0))
- fix(desktop): preserve timeline scroll when opening threads ([#1278](https://github.com/block/buzz/pull/1278)) ([`d05f122d`](https://github.com/block/buzz/commit/d05f122d8acf49e924fe11992dde4bde41dee3f7))
- chore: remove LLM-slop comments across the codebase ([#1277](https://github.com/block/buzz/pull/1277)) ([`73cc31cc`](https://github.com/block/buzz/commit/73cc31cc528318debedd38e85d67b79d1feb55e8))
- fix(desktop): allow saving personas with an empty system prompt ([#1276](https://github.com/block/buzz/pull/1276)) ([`996a3f89`](https://github.com/block/buzz/commit/996a3f89d5f3835009236782f2b3a93409b090cf))


## v0.3.33

- fix(desktop): always use legacy keyring for blob entry on macOS ([#1271](https://github.com/block/buzz/pull/1271)) ([`e7c3638fe`](https://github.com/block/buzz/commit/e7c3638fe984e7785a6e51e547b5844e894ed013))
- chore(release): release Buzz Relay version 0.1.1 ([#1269](https://github.com/block/buzz/pull/1269)) ([`68a0cc850`](https://github.com/block/buzz/commit/68a0cc8506be4ea1fb110b65eec0787e4cd84378))
- perf(desktop): consolidate keychain secrets into a single blob entry ([#1267](https://github.com/block/buzz/pull/1267)) ([`fa942cb51`](https://github.com/block/buzz/commit/fa942cb51d0714cba54747bcc7197c14e770c338))
- feat(desktop): re-snapshot persona config on every agent spawn ([#1268](https://github.com/block/buzz/pull/1268)) ([`048e8fdc0`](https://github.com/block/buzz/commit/048e8fdc004322e5feccc4fd76c8fc0dc87d91f0))
- feat(relay): add buzz-admin member management CLI with NIP-43 roster publish ([#1265](https://github.com/block/buzz/pull/1265)) ([`0cee0435f`](https://github.com/block/buzz/commit/0cee0435f7f955a42d7824f1967e4f4fc6a02f74))
- fix(desktop): fall back to old keychain when DPK unavailable (unsigned builds) ([#1266](https://github.com/block/buzz/pull/1266)) ([`958ac7aaa`](https://github.com/block/buzz/commit/958ac7aaaae90da76c864af744242f29ee4f08de))
- Align channel management panel with profile ([#1066](https://github.com/block/buzz/pull/1066)) ([`9f35b0188`](https://github.com/block/buzz/commit/9f35b018803f56b2330bfb79de4ab19f18d80256))
- fix(relay): multi-pod subscription coherence (one access-gated fan-out path + cross-pod cache invalidation + REQ/COUNT DB guard) ([#1261](https://github.com/block/buzz/pull/1261)) ([`628445429`](https://github.com/block/buzz/commit/6284454298cb5be72cefbeaf13abed04afe3cc9e))
- fix(desktop): switch macOS keychain to Data Protection Keychain ([#1264](https://github.com/block/buzz/pull/1264)) ([`35522311a`](https://github.com/block/buzz/commit/35522311a963b54f32df864d338ab7d75102247f))
- fix(desktop): use IPv4 loopback for media proxy URLs ([#1245](https://github.com/block/buzz/pull/1245)) ([`cee2c5f26`](https://github.com/block/buzz/commit/cee2c5f2634ae7d15262a2ec07cccacc8460cd0e))
- perf(relay/git): stream read-path, manifest info/refs fast path, idx sidecar (A/B/C) ([#1240](https://github.com/block/buzz/pull/1240)) ([`856815994`](https://github.com/block/buzz/commit/856815994c3441268eb48d2264aeb3492b701fe5))


## v0.3.32

- fix(desktop): restore solid dot for top-level channel unreads ([#1253](https://github.com/block/buzz/pull/1253)) ([`9d3bfd38b`](https://github.com/block/buzz/commit/9d3bfd38b0876cd1bdf5bb90d6639f200ceb12e0))
- Allow explicit root posts from reply prompts ([#1251](https://github.com/block/buzz/pull/1251)) ([`ebd74d11b`](https://github.com/block/buzz/commit/ebd74d11b4e77195fa94158205787e6e5edca3ff))
- feat: publish personas, teams, and managed agents as relay events ([#939](https://github.com/block/buzz/pull/939)) ([`d256935c3`](https://github.com/block/buzz/commit/d256935c3620f30ae54e9d02e48adc6ee0bc578f))
- fix(observability): gate agent observability on declared ownership, not key custody ([#1229](https://github.com/block/buzz/pull/1229)) ([`e993b9e74`](https://github.com/block/buzz/commit/e993b9e741649f73c24f9afbb01420cffc822e5a))
- fix(desktop): split AppShell notification effects ([#1248](https://github.com/block/buzz/pull/1248)) ([`a9ce477a0`](https://github.com/block/buzz/commit/a9ce477a041c7f201b1f7c0b79c68d7b0aa3f38e))
- feat(desktop): store nsec private keys in the OS keyring ([#1172](https://github.com/block/buzz/pull/1172)) ([`4be43d708`](https://github.com/block/buzz/commit/4be43d7086753e843884f5f541c3a8cbf1512fff))
- fix: add evict-completed-work prompt rule and unblock desktop file-size gate ([#1247](https://github.com/block/buzz/pull/1247)) ([`048228889`](https://github.com/block/buzz/commit/048228889122160ce73f9e32aba36961613e984e))
- fix(desktop): scope mention/add autocomplete to reachable identities ([#1243](https://github.com/block/buzz/pull/1243)) ([`a3e8fb6fd`](https://github.com/block/buzz/commit/a3e8fb6fd512f34d53b7ed36e68ab7f8bfa01291))
- test(e2e): raise scroll-history pagination test timeout to 90s ([#1234](https://github.com/block/buzz/pull/1234)) ([`09dbeb86f`](https://github.com/block/buzz/commit/09dbeb86fa8b6f430c79adf7c585194e2ec0a3d0))
- Polish thread and media layout ([#1239](https://github.com/block/buzz/pull/1239)) ([`b264a32cf`](https://github.com/block/buzz/commit/b264a32cfbdc8481e9dd4ceadbb05990bd7af02a))
- Polish desktop navigation chrome ([#1238](https://github.com/block/buzz/pull/1238)) ([`092546495`](https://github.com/block/buzz/commit/092546495d73d305e4d0226ee14bf0d5de4d2f53))
- fix: propagate persona harness edits to live agent instances ([#1244](https://github.com/block/buzz/pull/1244)) ([`2e426b2fd`](https://github.com/block/buzz/commit/2e426b2fddbbec1f1261125c81d83cd57e838fd1))
- fix(desktop): render autolinked message links as chips ([#1241](https://github.com/block/buzz/pull/1241)) ([`45067ec13`](https://github.com/block/buzz/commit/45067ec13507c406ee41d3f184fc96c988b1348b))
- fix(desktop): clear unread projections from message read markers ([#1242](https://github.com/block/buzz/pull/1242)) ([`733c766eb`](https://github.com/block/buzz/commit/733c766eba90733a00dc397dd2ab9706935e5183))
- fix(desktop): resolve repos_dir symlink at boot before agent restore ([#1231](https://github.com/block/buzz/pull/1231)) ([`86d7748eb`](https://github.com/block/buzz/commit/86d7748ebd93027754bfd2326958c4324aa817d6))
- Fix presence fan-out across relay pods ([#1227](https://github.com/block/buzz/pull/1227)) ([`36d3d2ed1`](https://github.com/block/buzz/commit/36d3d2ed1e57e399d5221edfd109082fd8a4d73e))
- fix(relay): raise grace limit, add replay backpressure, and NOTICE on oversized frames ([#1226](https://github.com/block/buzz/pull/1226)) ([`142a5c909`](https://github.com/block/buzz/commit/142a5c909542f1e1d2119ca4562129d492aca94c))
- [codex] Make relay frame limit configurable ([#1225](https://github.com/block/buzz/pull/1225)) ([`38a953343`](https://github.com/block/buzz/commit/38a95334396dae0e271a478b46baa62d29deebff))


## v0.3.31

- fix(release): publish versioned relay Docker tags via independent release lanes ([#1173](https://github.com/block/buzz/pull/1173)) ([`549b7d24`](https://github.com/block/buzz/commit/549b7d24813320045bdda629d865c6c7418e7450))
- fix(desktop): align settings section headers ([#1165](https://github.com/block/buzz/pull/1165)) ([`6ad68a6b`](https://github.com/block/buzz/commit/6ad68a6b095cd5db328ae07dfac4013eeda5820a))
- fix(desktop): ground agent workspace, migrate legacy nest, configurable repos_dir ([#1194](https://github.com/block/buzz/pull/1194)) ([`1011cea2`](https://github.com/block/buzz/commit/1011cea2682a5e8cd92c9da255d5ba6c8f7ced78))
- fix(desktop): enable mesh llm for release builds ([#1221](https://github.com/block/buzz/pull/1221)) ([`fa1262a9`](https://github.com/block/buzz/commit/fa1262a92c85b96c1d643c247d28df4f2f57e81a))
- fix(desktop): move crypto commands off the main thread ([#1222](https://github.com/block/buzz/pull/1222)) ([`e35e84b0`](https://github.com/block/buzz/commit/e35e84b08bdc44f1f5297e6957bcc52e7c31eb70))
- fix: tolerate missing private_key_nsec in agent store ([#1220](https://github.com/block/buzz/pull/1220)) ([`c58e9880`](https://github.com/block/buzz/commit/c58e9880bf60324b0fbb64917b6d1c8e197d4ea4))
- Update navigation header height ([#1212](https://github.com/block/buzz/pull/1212)) ([`6b5cf325`](https://github.com/block/buzz/commit/6b5cf325c2777f64696923cf5b1c1ffd4fdf82e2))
- Improve global search ([#1195](https://github.com/block/buzz/pull/1195)) ([`5130a6a0`](https://github.com/block/buzz/commit/5130a6a0b60c56a5c31549d3d4e85b956e35a671))
- Parse Typesense multi_search errors ([#1208](https://github.com/block/buzz/pull/1208)) ([`65ccb126`](https://github.com/block/buzz/commit/65ccb1262fb876b74584bca1165feef39eda67a6))
- fix(desktop): keep settings shortcut from opening search ([#1204](https://github.com/block/buzz/pull/1204)) ([`89ff9504`](https://github.com/block/buzz/commit/89ff950444d03ea09eb54661a21f0cb96f0bfcb6))
- Polish sidebar channel navigation ([#1213](https://github.com/block/buzz/pull/1213)) ([`c0a872e8`](https://github.com/block/buzz/commit/c0a872e898479bcb2c3dda1b642c0d1373174f68))
- fix(desktop): restore channel unread badges ([#1218](https://github.com/block/buzz/pull/1218)) ([`89aaa264`](https://github.com/block/buzz/commit/89aaa26443486244b6f004a7c419f4e0dc86aa44))
- Fix collapsed home header chrome overlap ([#1215](https://github.com/block/buzz/pull/1215)) ([`b4e75a1e`](https://github.com/block/buzz/commit/b4e75a1e41a614fa3449e814ccbd9f31090dfbfc))
- fix(desktop): dedupe welcome intro per channel ([#1216](https://github.com/block/buzz/pull/1216)) ([`2a522826`](https://github.com/block/buzz/commit/2a522826edc6dfb4df79f34256beea6b8597505b))
- fix(desktop): defer agent page secondary requests ([#1217](https://github.com/block/buzz/pull/1217)) ([`bee2d64c`](https://github.com/block/buzz/commit/bee2d64cf7f093088cc28463e96a6c94b64f280e))
- ci(release): enable Tauri auto-updater on Windows and Linux builds ([#1206](https://github.com/block/buzz/pull/1206)) ([`3ef2a8e5`](https://github.com/block/buzz/commit/3ef2a8e5c7e655f3347931135dde5f65b919c915))
- Hydrate reactions for rendered messages ([#1205](https://github.com/block/buzz/pull/1205)) ([`ed556f3d`](https://github.com/block/buzz/commit/ed556f3deb895e0adfa18274b3ed90f255b5f6ad))
- fix(desktop): show NIP-OA owners in profile pane ([#1198](https://github.com/block/buzz/pull/1198)) ([`40070a58`](https://github.com/block/buzz/commit/40070a58559938ed649950ccabce0725dd3c966e))
- fix(desktop): preserve login-shell PATH for managed agents ([#1193](https://github.com/block/buzz/pull/1193)) ([`29978b6f`](https://github.com/block/buzz/commit/29978b6f93cdd2c5d061093ddce87d567d8d4c17))
- Fix nav chrome offset in fullscreen ([#1192](https://github.com/block/buzz/pull/1192)) ([`b3b0704e`](https://github.com/block/buzz/commit/b3b0704efb5afc74dca0a093a6e4594973e9edf4))
- fix(desktop): show due-reminder count in the Inbox nav badge ([#1191](https://github.com/block/buzz/pull/1191)) ([`c0858dac`](https://github.com/block/buzz/commit/c0858dac12a3efd09d301f5424074f18df5cf422))


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
