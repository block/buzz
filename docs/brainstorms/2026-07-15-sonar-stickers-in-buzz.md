# Sonar Stickers in Buzz

Date: 2026-07-15

## Clarified Problem Statement

**Goal:** Add Sonar-compatible sticker packs to Buzz with a desktop-first install, author, import, edit, pick, send, and render experience, backed by a workspace-curated catalog and verified proxy/cache for public Blossom assets.

The choices captured here are:

- desktop-first implementation with shared protocol foundations and a later full mobile UX;
- Signal-link import and publishing plus native pack creation/editing;
- a workspace-curated catalog rather than global Sonar directory browsing;
- proxy/cache external Blossom assets while retaining the canonical hash-pinned Sonar reference.

The third answer was written as `1.B`; this brief interprets it as `3.B` because it follows the four-question order.

### Constraints

- Preserve the Sonar wire contract exactly for interoperable data:
  - kind `30031` addressable sticker packs;
  - kind `10031` per-user installed-pack lists;
  - `sonar-sticker-pack-v1` format marker;
  - sent `sticker` references containing pack coordinate, shortcode, and plaintext SHA-256.
- Use `core/sonar-stickers` from `hedwig-corp/bitchat-to-sonar` as the canonical Rust model, validator, Nostr converter, and Signal importer, pinned to an audited Git revision rather than a moving branch.
- Keep Buzz custom emoji (`30030`/`10030`) separate. Stickers are pack-scoped, visually larger, independently installed, and hash-pinned.
- Never publish, log, persist in events, or place in URLs a Signal `pack_key`. It is transient decryption input only.
- Accept only valid HTTPS, content-addressed sticker assets with allowlisted MIME, bounded dimensions/count/response size, and a verified plaintext hash.
- A proxy/cache request must identify a stored pack coordinate, shortcode, and hash. It must not be a generic fetch-by-URL endpoint. Redirects, DNS rebinding, private/link-local destinations, oversized bodies, MIME confusion, and hash mismatch must be rejected.
- Preserve channel scoping: sticker messages remain normal Buzz message events with the required `h` tag and thread metadata. The sticker reference is additional message metadata, not a new chat transport.
- Give unsupported/older clients a safe textual fallback such as `:shortcode:` or the sticker alt text. A missing, edited, or untrusted pack must render an explicit placeholder rather than a substituted asset.
- Pack edits reuse the same kind-`30031` coordinate. Existing messages never silently change meaning because resolution requires the original shortcode and plaintext hash.
- New workspace-scoped frontend caches must be reset through `resetWorkspaceState()`.
- The first release should include receive-only mobile rendering or a clear fallback so desktop-sent stickers do not produce blank messages. Mobile install, authoring, and picker parity can follow.

### Non-goals

- Browsing the global Sonar sticker directory or arbitrary external Nostr relays in the first release.
- Folding stickers into the existing custom-emoji palette or reaction system.
- Sticker reactions in the first release.
- A generic server-side URL proxy.
- Publishing Signal secret material or treating imported Signal packs as private.
- Full mobile pack management, Signal import, authoring, and picker UX in the desktop-first milestone.
- Browser-only Signal decryption or browser-only Blossom publishing.

### Success criteria

- Buzz can ingest, store, query, and replace valid `30031`/`10031` events and rejects malformed pack metadata at the relay boundary.
- The desktop catalog shows only approved workspace pack coordinates; unapproved but valid pack events are not discoverable through the curated surface.
- A user can install/uninstall a catalog pack, and their ordered selection round-trips through their Sonar-compatible kind-`10031` event.
- A desktop user can import a Signal sticker URL, author a pack from local images, edit owned pack metadata/assets, upload plaintext assets, and publish a valid kind-`30031` event.
- Native creation captures title, identifier, description, license, cover, sticker shortcode, alt text, representative emoji, and supported dimensions/MIME where applicable.
- A user can select a sticker in the composer and send a normal Buzz message containing the immutable Sonar sticker reference.
- Desktop and receive-capable mobile clients resolve only an exact coordinate/shortcode/hash match, render via the verified cache, and show a deterministic unavailable/untrusted state otherwise.
- Cached bytes are content-addressed and reused, while the signed Nostr event remains the authority. Pack metadata and cached authorization are cleared with workspace/identity state where required by the Sonar specification.
- `buzz stickers` provides agent-facing list/show/install/uninstall/import/create/update commands with compact JSON-compatible output and existing Buzz exit-code conventions.
- Unit, relay integration, desktop E2E, CLI, SSRF, hash-mismatch, Signal-HMAC, pack-edit, missing-pack, and mobile fallback tests pass; `just ci` passes before shipping.

## Approaches Considered

### Approach A: Sonar-native packs plus an explicit Buzz catalog

- **Sketch:** Keep `30031` packs and `10031` personal installed lists semantically pure. Add a Buzz-specific, Nostr-first curation command and relay-signed replaceable catalog event containing approved `a` coordinates. Creators can publish packs, but an owner/admin must approve a coordinate before it appears in the workspace catalog. Resolve assets through a coordinate-based lazy relay cache that validates the current pack and plaintext hash before fetching or serving bytes.
- **Affected files/modules:**
  - Rust dependency and protocol: `Cargo.toml`, `Cargo.lock`, `crates/buzz-core/src/kind.rs`, `crates/buzz-sdk/src/builders.rs`, `crates/buzz-sdk/src/lib.rs`.
  - Relay/catalog/cache: `crates/buzz-relay/src/handlers/ingest.rs`, a new sticker/catalog handler, `crates/buzz-relay/src/handlers/command_executor.rs` or the matching admin-command module, `crates/buzz-relay/src/api/media.rs`, and `crates/buzz-media/`.
  - Agent surface: `crates/buzz-cli/src/lib.rs`, `crates/buzz-cli/src/commands/stickers.rs`, `crates/buzz-cli/src/client.rs`.
  - Desktop Rust: `desktop/src-tauri/Cargo.toml`, new `commands/stickers.rs`, `commands/messages.rs`, `events.rs`, and asset-cache/proxy integration.
  - Desktop UI: a new `desktop/src/features/stickers/` feature, `desktop/src/features/settings/ui/SettingsPanels.tsx`, `desktop/src/features/messages/ui/MessageComposer.tsx`, `MessageComposerToolbar.tsx`, and the timeline message renderer.
  - Mobile receive path: `mobile/lib/shared/relay/nostr_models.dart`, `mobile/lib/features/channels/message_content.dart`, and focused widget tests.
- **Tradeoffs:** Cleanest semantics, explicit moderation, exact Sonar interoperability, and future multi-admin support. It adds a Buzz-specific catalog event/command and a security-sensitive verified cache path. The new kind numbers and owner/admin command authorization need a small protocol note.
- **Effort:** Large.

### Approach B: Treat the workspace owner's installed list as the catalog

- **Sketch:** Use only Sonar kinds. The current workspace owner's kind-`10031` list is interpreted by Buzz clients as the curated catalog; every other member's `10031` remains their personal installed list. Asset proxy/cache and all authoring/sending behavior remain the same as Approach A.
- **Affected files/modules:** The same client, CLI, SDK, pack-ingest, and media paths as Approach A, but no new catalog event or admin-command handler. Catalog reads additionally need reliable workspace-owner discovery.
- **Tradeoffs:** Smaller protocol surface and maximal Sonar purity. It overloads the owner's personal installed list with workspace policy, prevents ordinary admins from curating without access to the owner's signing key, and makes ownership transfer/catalog continuity awkward.
- **Effort:** Medium-to-large.

### Approach C: Every valid pack on the Buzz relay is discoverable

- **Sketch:** Allow members to publish validated kind-`30031` packs and define the workspace catalog as all current packs on that relay. Existing report/deletion/moderation flows remove abusive packs after publication. Keep `10031` personal and use the same verified cache and message reference design.
- **Affected files/modules:** Similar to Approach B. The catalog query is a direct `30031` fetch, while moderation needs pack-aware labels/actions in the existing queue.
- **Tradeoffs:** Simplest discovery model and easiest community contribution. It is community-published rather than genuinely curated, permits catalog spam until moderation occurs, and does not match the selected approval-oriented workspace catalog without additional policy.
- **Effort:** Medium-to-large.

## Recommendation

Choose **Approach A: Sonar-native packs plus an explicit Buzz catalog**. It preserves Sonar interoperability where interoperability matters—the pack, installed list, Signal import, and sent reference—while modeling Buzz's workspace curation as a separate concern instead of changing the meaning of kind `10031`.

Implement it in vertical stages: protocol/relay/CLI and verified cache first; desktop catalog/install/render; Signal import and native authoring/editing; composer send flow; then receive-only mobile compatibility. Full mobile management can follow without revisiting the wire format.

## Open questions

- Should both workspace owners and admins approve/remove catalog entries, or only the owner? Recommendation: both, through scoped Nostr admin commands.
- Which first-release message surfaces accept stickers: channel messages and threads only, or also DMs and forum posts? Recommendation: channel messages, threads, and DMs; defer forum root posts/comments unless the shared composer makes support effectively free.
- Should authors be allowed to remove a published sticker from an edited pack, knowing old hash-pinned messages will intentionally show unavailable? Recommendation: allow removal with a warning and preview of affected local references.
- Should catalog removal also evict cached bytes immediately, or only revoke authorization and let content-addressed garbage collection remove bytes later? Recommendation: revoke immediately, garbage-collect later.
- Is pack approval required before the relay performs any external fetch, or may authors preview an unapproved pack through a client-local verified fetch? Recommendation: local preview before approval; relay fetch/cache only after approval.

## References

- Sonar specification: <https://github.com/hedwig-corp/bitchat-to-sonar/blob/main/docs/SONAR-STICKERS.md>
- Reference Rust crate: <https://github.com/hedwig-corp/bitchat-to-sonar/tree/main/core/sonar-stickers>
