# Native secp256k1

`vendor/src`, `vendor/include`, and `vendor/COPYING` are unmodified files from
Bitcoin Core libsecp256k1 v0.8.0, commit
6e2c8bc4ecdc6e71dbe7a368f360d8d453ce435d:
https://github.com/bitcoin-core/secp256k1/tree/6e2c8bc4ecdc6e71dbe7a368f360d8d453ce435d

The upstream MIT license is retained in `vendor/COPYING`. SwiftPM compiles the
three translation units listed in Package.swift, with the extrakeys and
schnorrsig modules enabled. Upstream precomputed tables are checked in; no
build plugin, downloaded binary, or generated source is needed.

To update, replace these three vendor paths from a reviewed upstream release.
Do not edit upstream sources locally. Run the NostrSchnorr BIP-340 vector tests,
all BuzzPushKit tests, and complete app and notification extension builds on
supported Xcode versions before accepting an update.

The BIP-340 test fixture comes from bitcoin/bips commit
927b6de9915c9262615a6399de51b200f81e5aa4, bip-0340/test-vectors.csv.
