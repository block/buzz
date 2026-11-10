# Provider wire fixtures

The shared arbiter for the stdin/stdout contract between the desktop
(`agents_deploy.rs`) and this provider (spec §Provider Protocol).

Each `*.request.json` is a request the desktop can emit; each matching
`*.response.json` is the exact response this provider produces for it. The
provider side is asserted by `tests/wire_fixtures.rs`; the desktop side should
assert that its emitted payloads parse as the corresponding request.

Three rules keep these useful rather than decorative:

* **Requests are recorded, not invented.** A fixture that no caller emits
  tests a contract nobody has. "Recorded" means *executed and transcribed* —
  `deploy-full-launch.request.json` is the output of the desktop's real
  `build_launch_block` → `deploy_payload_json` path, not a shape derived by
  reading those functions. Deriving it is how this fixture acquired four
  impossible values at once: a `respond_to` that was a pubkey where the
  desktop serializes a kebab-case `RespondTo` enum, allowlist and owner
  values failing `validate_respond_to_allowlist`'s 64-hex rule
  (`types.rs:897`), an invented `BUZZ_ACP_PARALLELISM` where the emitter
  writes `BUZZ_ACP_AGENTS` (`runtime.rs:729`), and a `launch.env` key from
  no layer of `resolve_effective_harness_descriptor`.
* **The provider cannot police this file, so the desktop must.** Every field
  above is one this provider is deliberately indifferent to — `respond_to` is
  an opaque `Option<String>`, the allowlist an opaque `Vec<String>`,
  `policy_env` an arbitrary map — so `the_full_desktop_payload_is_accepted`
  passes on invented data exactly as happily as on recorded data. The
  enforcement is the desktop's whole-object equality test, which *builds* the
  payload and compares it to this file. A completeness guard (the case-list
  directory scan in `wire_fixtures.rs`) stops a case from going missing; it
  cannot tell you a case is false.
* **Responses are byte-compared after key-sorted re-serialization**, so a
  field rename or a type change fails here rather than in a desktop that
  silently reads `undefined`.

`deploy-*` fixtures cover only responses reachable without a cluster —
refusals and malformed input. A successful deploy needs an apiserver and is
covered by the conformance suite, not by a static fixture.
