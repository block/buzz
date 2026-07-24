# Changelog

## mobile-v0.4.11

- fix(mobile): stop media fetch stampede — stable cache keys, bounded decode, 429 cooldown ([#2219](https://github.com/block/buzz/pull/2219)) ([`591bbbfa7`](https://github.com/block/buzz/commit/591bbbfa7e5931318a226555d6bf921ba50d97c2))


## mobile-v0.4.9

- fix(mobile): sanitize Android image uploads ([#2188](https://github.com/block/buzz/pull/2188)) ([`ee21da90b`](https://github.com/block/buzz/commit/ee21da90bd6b1da6bfaaf22ba00749398aaa9640))
- chore(release): release Buzz Mobile version 0.4.8 ([#2187](https://github.com/block/buzz/pull/2187)) ([`fb8c90cf5`](https://github.com/block/buzz/commit/fb8c90cf597960e48fe55a9baceb8a5a08112ccc))
- fix(mobile): image upload fails due to unstripped metadata ([#2185](https://github.com/block/buzz/pull/2185)) ([`37f15b200`](https://github.com/block/buzz/commit/37f15b20019169363b697aee41c99573b7bc3f24))
- chore(deps): update all non-major dependencies ([#2152](https://github.com/block/buzz/pull/2152)) ([`6da407742`](https://github.com/block/buzz/commit/6da407742a09ee531c53a99ca5d2d314f0b38aef))


## mobile-v0.4.8

- fix(mobile): image upload fails due to unstripped metadata ([#2185](https://github.com/block/buzz/pull/2185)) ([`37f15b200`](https://github.com/block/buzz/commit/37f15b20019169363b697aee41c99573b7bc3f24))
- chore(deps): update all non-major dependencies ([#2152](https://github.com/block/buzz/pull/2152)) ([`6da407742`](https://github.com/block/buzz/commit/6da407742a09ee531c53a99ca5d2d314f0b38aef))


## mobile-v0.4.7

- fix(ui): relabel agent owner attribution from "owned by" to "managed by" ([#2133](https://github.com/block/buzz/pull/2133)) ([`a1e977cd1`](https://github.com/block/buzz/commit/a1e977cd1bda3f5421533b47f12c38f4a336224e))
- chore(release): release Buzz Mobile version 0.4.6-rc.1 ([#2049](https://github.com/block/buzz/pull/2049)) ([`3aba3a531`](https://github.com/block/buzz/commit/3aba3a5316ae8bb1aed518e01e68c3e3a2166a46))
- Strip media metadata on clients and reject it at the relay ([#2006](https://github.com/block/buzz/pull/2006)) ([`5cfd69cb0`](https://github.com/block/buzz/commit/5cfd69cb0cf1dc63d718454defe3b8a8aaf5f15b))


## mobile-v0.4.6-rc.1

- Strip media metadata on clients and reject it at the relay ([#2006](https://github.com/block/buzz/pull/2006)) ([`5cfd69cb`](https://github.com/block/buzz/commit/5cfd69cb0cf1dc63d718454defe3b8a8aaf5f15b))


## mobile-v0.4.5

- fix(mobile): support open pairing relays ([#1939](https://github.com/block/buzz/pull/1939)) ([`a0081944`](https://github.com/block/buzz/commit/a0081944edbb3d33a2bcee0b4890ae5ef8ad4966))
- fix(mobile): replace pairing placeholder with Buzz icon ([#1952](https://github.com/block/buzz/pull/1952)) ([`7eea924f`](https://github.com/block/buzz/commit/7eea924f3d7cf9ee816ae5f206c88762274fd7c6))
- feat(media): require auth for relay media reads ([#1926](https://github.com/block/buzz/pull/1926)) ([`f3087628`](https://github.com/block/buzz/commit/f3087628524951de91028c9d263bcd0d0a727fab))


## mobile-v0.4.4

- fix(mobile): surface non-member people and owned agents in @mention autocomplete ([#1877](https://github.com/block/buzz/pull/1877)) ([`54f41659`](https://github.com/block/buzz/commit/54f4165942ab5b3e2cf1fa1b47bbb907587cd5f4))
- fix(mobile): open profile sheet when tapping @mentions ([#1876](https://github.com/block/buzz/pull/1876)) ([`7e009a89`](https://github.com/block/buzz/commit/7e009a893748eb660efd6d51d78f172cc913e3da))
- fix(mobile): handle buzz:// links ([#1826](https://github.com/block/buzz/pull/1826)) ([`37f65c08`](https://github.com/block/buzz/commit/37f65c08dc4fc357e54283b19d6019b18cd2b00e))


## mobile-v0.4.3

- fix(mobile): fix agent avatars on mobile ([#1837](https://github.com/block/buzz/pull/1837)) ([`7b6e1265`](https://github.com/block/buzz/commit/7b6e12657d69316614726065db69cc418750a973))
- refactor(clients): standardize product naming on community ([#1858](https://github.com/block/buzz/pull/1858)) ([`3e76481a`](https://github.com/block/buzz/commit/3e76481a149bb3de459298cc989505c470c2372c))
- fix(mobile): mirror app bar title padding when actions are empty ([#1832](https://github.com/block/buzz/pull/1832)) ([`f3599f2c`](https://github.com/block/buzz/commit/f3599f2cd4509e9120d8466d0ff2f79ee9dc5803))
- docs(mobile): backfill mobile changelog ([#1835](https://github.com/block/buzz/pull/1835)) ([`259a1724`](https://github.com/block/buzz/commit/259a1724d31787a8a110e2689b8a813b22d0a382))
- BOT-1247 Configure Android Play identity and signing ([#1829](https://github.com/block/buzz/pull/1829)) ([`f1706e23`](https://github.com/block/buzz/commit/f1706e23f79020812ef097c3167102a8ee3bfc9a))


## mobile-v0.4.2

- fix(mobile): add mentioned agents to channels ([#1696](https://github.com/block/buzz/pull/1696)) ([`3112e59f`](https://github.com/block/buzz/commit/3112e59fd8141818712b04d55919145f0516846b))
- Resynchronize the mobile version with the canonical release line after an accidental desktop-derived 0.4.1 version ([#1833](https://github.com/block/buzz/pull/1833)) ([`ffa2de0f`](https://github.com/block/buzz/commit/ffa2de0fba94438397429da7bf60e774285c7ddd))


## mobile-v0.3.33

- fix(mobile): align Android AGP/Gradle/Kotlin with Flutter 3.41.7 (unbreak Android build) ([#1775](https://github.com/block/buzz/pull/1775)) ([`bc14052d`](https://github.com/block/buzz/commit/bc14052d3941ffcc4b54541f65b24b68ce8f215b))
- Fix mobile relay reconnect lifecycle ([#1772](https://github.com/block/buzz/pull/1772)) ([`dbe7fb85`](https://github.com/block/buzz/commit/dbe7fb853486fe33d9df2660b29dee4574007cd1))
- Serialize managed agent PATH tests ([#1777](https://github.com/block/buzz/pull/1777)) ([`1fc7488e`](https://github.com/block/buzz/commit/1fc7488ea58accf8bd96afe94cd93ee54174df00))
- Fix oversized mobile Manage channel sheet ([#1774](https://github.com/block/buzz/pull/1774)) ([`81b55c9f`](https://github.com/block/buzz/commit/81b55c9f3eb147cf2b5753232ff75df6e51431f2))
- fix(mobile): highlight full multi-word mentions ([#1762](https://github.com/block/buzz/pull/1762)) ([`92bb959e`](https://github.com/block/buzz/commit/92bb959ee1580ed05a45b80af599b53d78f611bb))
- [BOT-1264] fix(mobile): #channel tags not tappable ([#1695](https://github.com/block/buzz/pull/1695)) ([`2687a2a1`](https://github.com/block/buzz/commit/2687a2a185a01bfb08478267587c08378d60136d))


## mobile-v0.3.32

- Restrict the iOS app to iPhone ([#1735](https://github.com/block/buzz/pull/1735)) ([`215f5218`](https://github.com/block/buzz/commit/215f521880b29d421221b371b5a5818fcdaf4dea))


## mobile-v0.3.31

- This release only resynchronized the pre-1.0 mobile version after an accidental 1.0.0 internal build ([#1724](https://github.com/block/buzz/pull/1724)) ([`bdb3e698`](https://github.com/block/buzz/commit/bdb3e6989988c2721e24ea37236edd60038eca50))
