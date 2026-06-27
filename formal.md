# Formal Methods Audit

Reviewed surfaces:

- `docs/spec/GitOnObjectStore.tla` and `docs/git-on-object-storage.md`
- `crates/buzz-core/src/pairing/NIP-AB.spthy` and `crates/buzz-core/src/pairing/NIP-AB.md`
- The Git relay implementation in `crates/buzz-relay/src/api/git/` and `crates/buzz-relay/src/handlers/`
- The NIP-AB Rust state machine / CLI and the mobile target implementation

## Findings

### High: rejected and no-op Git pushes still advance the manifest pointer

The TLA model has an explicit no-op path: `MustPublish(p)` is false when refs did not change and snapshots succeeded, and `SkipPublish(p)` leaves `pointer` and `published` unchanged (`docs/spec/GitOnObjectStore.tla:86-120`). The prose makes the same claim twice: rejected or no-op pushes never reach the CAS fence (`docs/git-on-object-storage.md:203-221`) and no-op pushes pay no kind:30618 cost (`docs/git-on-object-storage.md:443-446`).

The handler does not implement that path. `receive_pack()` always wraps the `receive-pack` output in a `PushContext` and calls `finalize_push()` (`crates/buzz-relay/src/api/git/transport.rs:511-525`). `run_git_at()` logs a failing Git subprocess but still returns `Ok(PackOutput)` because protocol errors are carried in-band (`crates/buzz-relay/src/api/git/transport.rs:612-620`). `finalize_push()` then unconditionally calls `cas_publish()` (`crates/buzz-relay/src/api/git/transport.rs:674-735`).

Even if refs, HEAD, and packs are unchanged, `compose_after()` writes the current manifest digest into the new manifest's `parent` field (`crates/buzz-relay/src/api/git/cas_publish.rs:356-377`). That makes the child manifest bytes differ from the parent manifest bytes, so the CAS advances the pointer and `manifest_changed` is true (`crates/buzz-relay/src/api/git/transport.rs:737-760`). The comment that equal published state implies equal digest is false once `parent` is part of canonical bytes.

This is security-relevant because HTTP auth only gates relay membership; repo write authorization is deferred to the pre-receive hook (`crates/buzz-relay/src/api/git/transport.rs:49-51`, `crates/buzz-relay/src/api/git/transport.rs:177-190`). A relay member whose push is rejected by policy can still mutate the authoritative pointer history and force concurrent legitimate writers to lose CAS. The proof transfer fails exactly at the modeled `SkipPublish` branch.

### High: shipped NIP-AB target code exposes payload plaintext before dual consent

The NIP-AB spec requires the target not to deserialize, extract, persist, or act on the `payload` field until both transcript verification and local SAS approval have completed (`crates/buzz-core/src/pairing/NIP-AB.md:316-318`). The formal section claims the model proves an even stronger property: plaintext is not made available to protocol logic before both conditions (`crates/buzz-core/src/pairing/NIP-AB.md:645-649`).

The mobile target violates that boundary directly. `_handlePairingEvent()` decrypts every peer event and immediately `jsonDecode`s the full plaintext map (`mobile/lib/features/pairing/pairing_provider.dart:311-318`). When a payload arrives after transcript verification but before the user taps approval, `_handlePayload()` stores the full decoded map in `_pendingPayload` (`mobile/lib/features/pairing/pairing_provider.dart:380-388`). At that point the `payload` field has already been extracted and retained in application memory.

The Rust reference path also has an early-deserialization route. `decrypt_message()` always deserializes the whole JSON object into `PairingMessage`, whose payload variant contains a normal `String` (`crates/buzz-core/src/pairing/session.rs:600-632`, `crates/buzz-core/src/pairing/types.rs:39-45`). `handle_abort()` calls `decrypt_message()` before it knows whether the event is an abort (`crates/buzz-core/src/pairing/session.rs:455-480`), and the CLI intentionally probes every inbound event through `handle_abort()` first (`crates/buzz-pairing-cli/src/main.rs:273-295`, `crates/buzz-pairing-cli/src/main.rs:414-423`). If an adversarial relay delivers the immediately-following payload before the CLI has accepted the preceding `sas-confirm`, the reference target parses the secret before transcript verification or local approval.

The formal dual-consent result therefore does not describe either shipped target implementation. The issue is not just that import is delayed; the secret material is already present in ordinary heap objects before the user has approved it.

### Medium: the Tamarin buffering rule already binds plaintext before approval

The Tamarin comments say `Target_Buffers_Payload` receives ciphertext "WITHOUT extracting the plaintext" and that only `Target_Decrypts_Payload` performs symbolic decryption (`crates/buzz-core/src/pairing/NIP-AB.spthy:200-223`, `crates/buzz-core/src/pairing/NIP-AB.spthy:235-252`). The rule itself does not match that description:

```tamarin
In(< 'payload_evt', senc(msg, h(< 'pair-key', pkS^xt >)) >)
```

That premise binds `msg` in the buffering rule (`crates/buzz-core/src/pairing/NIP-AB.spthy:224-233`). In a symbolic model, destructuring `senc(msg, key)` while the rule has the key is already the point where plaintext becomes available to that rule. The later `Target_Decrypts_Payload` rule only introduces an action fact named `TargetDecryptedPayload`.

As a result, `target_decrypts_payload_only_after_dual_consent` proves that a particular action label occurs after consent (`crates/buzz-core/src/pairing/NIP-AB.spthy:383-394`); it does not prove the stronger prose claim that protocol logic cannot access plaintext before consent. The model needs to buffer an opaque ciphertext term and only destructure it in the post-approval rule if it wants to justify that claim.

### Medium: `Inv_Closed` does not prove the stated Git reconstruction theorem

The prose theorem says a reader can reconstruct every object reachable from every published ref (`docs/git-on-object-storage.md:265-299`). The TLA model has no relation between a ref value and the pack that contains that object. `refs[m]` is an arbitrary object id, while `packs[m]` is independently built as the parent pack set plus the manifest id (`docs/spec/GitOnObjectStore.tla:22-43`, `docs/spec/GitOnObjectStore.tla:92-112`).

`Inv_Closed` only checks that a child manifest keeps its parent's pack ids and adds its own id (`docs/spec/GitOnObjectStore.tla:212-219`). The model can choose a `refs[m]` value unrelated to every named pack, and no invariant rejects that state. TLC is therefore checking monotone pack-set inheritance, not object-graph coverage.

This matters because the read path explicitly skips stronger Git connectivity checks on the basis that `Inv_Closed` already proves coverage (`crates/buzz-relay/src/api/git/hydrate.rs:209-213`). A bug in pack capture or manifest construction that drops a reachable object is outside the model but inside the implementation's trust boundary.

### Medium: the "announced iff pointer exists" Git invariant is only a best-effort side effect

The Git design says kind:30617 seeding happens before the announcement is published, so pointer absence means "never announced" (`docs/git-on-object-storage.md:486-495`). The handler comments repeat that invariant (`crates/buzz-relay/src/handlers/side_effects.rs:1968-1972`).

In the ingest pipeline, the event is inserted first, then side effects run, and any side-effect error is only logged before the stored event is fanned out and accepted (`crates/buzz-relay/src/handlers/ingest.rs:1774-1862`). `handle_git_repo_announcement()` can fail for an invalid `d` tag, a name collision, a quota violation, or a pointer-seeding failure (`crates/buzz-relay/src/handlers/side_effects.rs:1871-1981`). Those failures therefore leave a stored kind:30617 without the reservation or pointer that the proof correspondence assumes.

The write path does not fail closed on that state. `hydrate_for_write()` treats a missing pointer as a fresh repository (`crates/buzz-relay/src/api/git/hydrate.rs:109-145`), while the pre-receive policy resolves repository existence from the stored kind:30617 event (`crates/buzz-relay/src/api/git/policy.rs:239-268`). This creates an implementation state outside the documented initialization invariant and weakens the global name-reservation guarantee.

### Medium: the Rust target cannot send the required `sas_mismatch` abort after transcript failure

The spec requires the target to send `abort` with reason `sas_mismatch` when transcript verification fails (`crates/buzz-core/src/pairing/NIP-AB.md:316`, `crates/buzz-core/src/pairing/NIP-AB.md:447-453`, `crates/buzz-core/src/pairing/NIP-AB.md:753-758`).

`handle_sas_confirm()` sets the session state to `Aborted` before returning `PairingError::TranscriptMismatch` (`crates/buzz-core/src/pairing/session.rs:362-371`). `abort()` rejects calls from the `Aborted` state (`crates/buzz-core/src/pairing/session.rs:432-452`). The CLI catches `TranscriptMismatch` and tries to call `session.abort(SasMismatch)`, but that call can no longer produce an event (`crates/buzz-pairing-cli/src/main.rs:281-292`).

The Tamarin model explicitly abstracts away abort branches (`crates/buzz-core/src/pairing/NIP-AB.md:656-670`), so this protocol-compliance failure is invisible to the proof. The source side receives no explicit security abort and can only time out.

### Low: the mobile target does not clear retained pairing secrets on terminal cleanup

The NIP-AB spec requires ephemeral private keys, session secrets, and decrypted payload plaintext to be zeroed on completion, abort, or timeout (`crates/buzz-core/src/pairing/NIP-AB.md:389`, `crates/buzz-core/src/pairing/NIP-AB.md:555-562`).

The mobile notifier keeps `_ephemeralPrivkey`, `_sessionSecret`, `_sasInput`, `_conversationKey`, and related session fields as instance members (`mobile/lib/features/pairing/pairing_provider.dart:132-143`). `_cleanup()` cancels timers, disposes the socket, clears flags, and nulls `_pendingPayload`, but it does not zero or even clear those secret fields (`mobile/lib/features/pairing/pairing_provider.dart:119-128`).

That leaves sensitive material referenced after terminal states until the notifier itself is collected. The Rust session has an explicit `Drop` zeroization path (`crates/buzz-core/src/pairing/session.rs:740-749`); the mobile implementation does not have an equivalent.

## Verification

`GitOnObjectStore.tla` still passes its checked-in TLC configuration:

```text
1435102 states generated, 435745 distinct states found, 0 states left on queue.
Model checking completed. No error has been found.
```

I could not rerun the Tamarin proof locally because the installed `maude` binary fails to start: it is linked against a missing `/opt/homebrew/opt/libtecla/lib/libtecla.1.6.3.dylib`.
