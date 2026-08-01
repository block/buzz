# Command Adviser Harness Packaging Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore the real ACP harness executables in Command Adviser and make
future local release builds fail before installation if any required sidecar is
missing, empty, or non-executable.

**Architecture:** Keep Tauri's zero-byte stubs for compile-only development
checks, but prohibit them in `desktop-release-build`. The release recipe will
compile the existing Rust sidecar packages, use the repository's established
`bundle-sidecars.sh` path, validate both the staged sidecars and final app
bundle, then install the rebuilt application without changing live data.

**Tech Stack:** Bash, Just, Cargo/Hermit, Tauri 2, macOS codesign.

## Global Constraints

- Preserve all existing Command Adviser user data, configuration, identities,
  Keychain entries, Battle Rhythm records, Plans, conversations, and agents.
- Do not replace `/Applications/Command Adviser.app` until the rebuilt bundle
  passes sidecar, signature, and entitlement checks.
- The six required Tauri sidecars are `buzz-acp`, `buzz-agent`,
  `buzz-lmstudio-agent`, `buzz-dev-mcp`, `git-credential-nostr`, and `buzz`.

---

### Task 1: Add a release-sidecar regression gate

**Files:**
- Create: `scripts/tests/verify-desktop-sidecars-test.sh`
- Create: `scripts/verify-desktop-sidecars.sh`
- Modify: `scripts/bundle-sidecars.sh`

**Interfaces:**
- Produces: `scripts/verify-desktop-sidecars.sh <directory> [target-triple]`
  with exit zero only when every expected sidecar exists, is non-empty, and is
  executable on Unix targets.

- [x] Write a shell regression test covering valid, zero-byte, non-executable,
  and missing sidecars plus the release-recipe integration.
- [x] Run the test and confirm it fails because the verifier and recipe
  integration do not exist.
- [x] Implement the verifier and invoke it after `bundle-sidecars.sh` copies
  release artifacts.
- [x] Rerun the focused regression test and confirm it passes.

### Task 2: Correct the local release recipe

**Files:**
- Modify: `Justfile`

**Interfaces:**
- Consumes: the existing Cargo packages and `bundle-sidecars.sh`.
- Produces: `just desktop-release-build <target>` with real sidecars in both
  `desktop/src-tauri/binaries` and the final application bundle.

- [x] Replace release-time `touch` commands with the same Cargo build and
  bundling sequence used by the upstream release workflow.
- [x] Validate staged sidecars before Tauri and bundled sidecars after Tauri.
- [x] Run the focused regression test, formatting checks, and shell syntax
  checks.

### Task 3: Rebuild, install, and exercise the live harness

**Files:**
- Modify: `docs/testing/upstream-v0.5.2-harness-repair.md`

**Interfaces:**
- Produces: a signed `/Applications/Command Adviser.app` with functional ACP
  sidecars and recorded live evidence.

- [x] Build `aarch64-apple-darwin` through the corrected recipe.
- [x] Verify all packaged sidecars are non-empty/executable, then sign and
  verify the app and its entitlements.
- [x] Copy the currently installed broken application to a timestamped,
  recoverable backup and replace only the app bundle.
- [ ] Launch Command Adviser and confirm at least one configured managed agent
  starts `buzz-acp` successfully without the prior not-found error.
- [x] Confirm the relay is healthy and existing live record counts remain
  unchanged.
- [x] Record exact evidence, commit, and push the repair to the existing draft
  upstream-sync PR.
