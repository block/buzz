# Autonomous Agent Skills Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give each Command Adviser specialist a small, autonomous, reversible skill-learning loop that turns repeated successful work into an encrypted, versioned `SKILL.md`, activates it only after deterministic checks, and rolls it back after verified regressions.

**Architecture:** Buzz remains authoritative. Two owner-encrypted, agent-authored Nostr contracts hold immutable skill versions and one replaceable active pointer; a local SQLite cache provides crash recovery and a materializer projects only the verified active version into the existing `.agents/skills` loader. The first usable learner is deliberately bounded: two successful turns with the same normalized task pattern create a deterministic candidate, promotion occurs between turns after fixed validation, and two matching failed turns roll back to the last passing parent.

**Tech Stack:** Rust, Nostr/NIP-44, SQLite/rusqlite, Buzz relay HTTP bridge, existing `buzz-agent` `SKILL.md` discovery, shell acceptance scripts.

## Global Constraints

- Use event kinds `30180` for immutable skill versions and `30181` for the addressable active pointer; both are encrypted to the owner and use the existing agent-or-owner read boundary.
- The v1 learner supports text-only `SKILL.md` bundles of at most 32 KiB; supporting-file objects remain out of scope until evidence shows they are needed.
- Candidate creation and promotion need no owner prompt, but a skill cannot add tools, network endpoints, credentials, provider policy, model files, release configuration, or external-action authority.
- Existing user-authored skill directories are never overwritten or deleted. Managed skills use the reserved `learned-<12 lowercase hex>` directory prefix.
- Signed events are authoritative. SQLite and `.agents/skills/learned-*` are disposable derived state and must rebuild from verified relay events.
- A turn keeps the skill snapshot it started with. Promotion or rollback invalidates the ACP session so the next turn performs a fresh skill scan.
- Candidate generation is deterministic in v1; it does not ask the model to judge its own work.
- All test and git commands run after `. ./bin/activate-hermit` in the same shell.

---

## File Map

- `crates/buzz-core/src/agent_skill.rs` — bounded payloads, NIP-44 encryption, event construction, decryption, hashes, and validation.
- `crates/buzz-core/src/agent_skill_tests.rs` — contract, privacy, tamper, lineage, and size tests.
- `crates/buzz-core/src/kind.rs` — allocate kinds `30180` and `30181` and register their privacy semantics.
- `crates/buzz-relay/src/handlers/ingest.rs` — admit only canonical skill envelopes.
- `crates/buzz-relay/src/handlers/req.rs` — apply the existing agent-or-owner filter gate to skill events.
- `crates/buzz-relay/src/handlers/event.rs` and `crates/buzz-relay/src/api/bridge.rs` — apply result-level and HTTP read gates consistently.
- `crates/buzz-search/tests/fts_integration.rs` plus schema/migration policy lists — prove encrypted skill events cannot enter FTS.
- `crates/buzz-acp/src/skill_learning/mod.rs` — runtime lifecycle and the between-turn promotion/rollback decision.
- `crates/buzz-acp/src/skill_learning/registry.rs` — SQLite observations, immutable versions, active pointers, evaluations, and durable publication outbox.
- `crates/buzz-acp/src/skill_learning/candidate.rs` — normalized workflow key and deterministic candidate content.
- `crates/buzz-acp/src/skill_learning/evaluate.rs` — schema, policy, inherited-test, replay, and regression checks.
- `crates/buzz-acp/src/skill_learning/materialize.rs` — hash-verified atomic projection into `.agents/skills/learned-*`.
- `crates/buzz-acp/src/skill_learning/rebuild.rs` — relay query, signature/decryption validation, active-head selection, and cache reconstruction.
- `crates/buzz-acp/src/skill_learning_tests.rs` — end-to-end local state-machine tests.
- `crates/buzz-acp/src/experience_capture.rs` and `crates/buzz-acp/src/pool.rs` — retain the bounded task text and feed terminal outcomes to the learner.
- `crates/buzz-agent/src/hints.rs` — reject malformed or unverifiable managed skill markers while preserving ordinary skills.
- `scripts/check-autonomous-skills.sh` — repeatable focused gate.
- `docs/command-console/autonomous-skills-operations.md` — status, rebuild, rollback behaviour, paths, and acceptance procedure.

---

### Task 1: Define the encrypted skill contracts and relay boundary

**Files:**
- Create: `crates/buzz-core/src/agent_skill.rs`
- Create: `crates/buzz-core/src/agent_skill_tests.rs`
- Modify: `crates/buzz-core/src/lib.rs`
- Modify: `crates/buzz-core/src/kind.rs`
- Modify: `crates/buzz-relay/src/handlers/ingest.rs`
- Modify: `crates/buzz-relay/src/handlers/req.rs`
- Modify: `crates/buzz-relay/src/handlers/event.rs`
- Modify: `crates/buzz-relay/src/api/bridge.rs`
- Test: `crates/buzz-search/tests/fts_integration.rs`

**Interfaces:**
- Produces: `SkillScope`, `SkillTestV1`, `SkillVersionV1`, `SkillPointerReason`, `SkillPointerV1`, `build_skill_version_event`, `build_skill_pointer_event`, `validate_and_decrypt_skill_version`, and `validate_and_decrypt_skill_pointer`.
- Produces: `KIND_AGENT_SKILL_VERSION = 30180` and `KIND_AGENT_SKILL_POINTER = 30181`.

- [ ] **Step 1: Write failing core tests**

  Cover a valid private version, a valid team-shared version, unique version addressing, stable pointer addressing, round-trip encryption, wrong-owner rejection, hash mismatch, invalid parent, self-parent, unknown state, oversize `SKILL.md`, duplicate inherited checks, forbidden required tools, and altered ciphertext/signature.

- [ ] **Step 2: Run the focused tests and confirm the new symbols are absent**

  Run:

  ```bash
  . ./bin/activate-hermit && cargo test -p buzz-core agent_skill
  ```

  Expected: compilation fails because `agent_skill` and the two kind constants do not exist.

- [ ] **Step 3: Implement the minimal bounded contract**

  Use these public shapes:

  ```rust
  pub enum SkillScope { SpecialistPrivate, CommandTeamShared }

  pub struct SkillTestV1 {
      pub check_id: String,
      pub kind: String,
      pub expected: String,
  }

  pub struct SkillVersionV1 {
      pub skill_id: String,
      pub version_id: String,
      pub parent_version_id: Option<String>,
      pub scope: SkillScope,
      pub specialist_id: Option<String>,
      pub team_id: Option<String>,
      pub created_at: String,
      pub source_experience_ids: Vec<String>,
      pub required_tools: Vec<String>,
      pub inherited_tests: Vec<SkillTestV1>,
      pub regression_tests: Vec<SkillTestV1>,
      pub skill_md: String,
      pub content_hash: String,
  }

  pub enum SkillPointerReason { Promotion, Rollback }

  pub struct SkillPointerV1 {
      pub skill_id: String,
      pub active_version_id: String,
      pub previous_version_id: Option<String>,
      pub scope: SkillScope,
      pub specialist_id: Option<String>,
      pub team_id: Option<String>,
      pub changed_at: String,
      pub reason: SkillPointerReason,
      pub evaluation_ids: Vec<String>,
  }
  ```

  Serialize with `deny_unknown_fields`, validate RFC3339 timestamps and lowercase identifiers, cap lists at 64 entries, cap `skill_md` at `MAX_SKILL_BODY_BYTES`, and compute SHA-256 over the exact UTF-8 bytes. Encrypt with NIP-44 v2 to the owner. Derive version and pointer `d` tags with separate versioned HMAC domains so neither collides with NIP-AE memory.

- [ ] **Step 4: Add fail-closed relay admission and read gates**

  Require exactly one lowercase 64-hex `d` tag and one lowercase owner `p` tag for both kinds. Admit agent-authored writes under `UsersWrite`, store them globally, exclude them from FTS, and require every enumerating filter to identify either the authenticated agent author or authenticated owner `#p`. Direct-id lookup, COUNT, live fan-out, and both HTTP query surfaces must apply the same event-level owner gate.

- [ ] **Step 5: Run the core, relay, and search tests**

  ```bash
  . ./bin/activate-hermit && \
    cargo test -p buzz-core agent_skill && \
    cargo test -p buzz-relay agent_skill && \
    cargo test -p buzz-search p_gated
  ```

  Expected: all focused tests pass and encrypted skill plaintext is absent from search results.

- [ ] **Step 6: Commit the contract**

  ```bash
  git add crates/buzz-core crates/buzz-relay crates/buzz-search
  git commit -s -m "feat: add encrypted agent skill contracts"
  ```

---

### Task 2: Build the deterministic learner, validator, and durable registry

**Files:**
- Create: `crates/buzz-acp/src/skill_learning/mod.rs`
- Create: `crates/buzz-acp/src/skill_learning/registry.rs`
- Create: `crates/buzz-acp/src/skill_learning/candidate.rs`
- Create: `crates/buzz-acp/src/skill_learning/evaluate.rs`
- Create: `crates/buzz-acp/src/skill_learning_tests.rs`
- Modify: `crates/buzz-acp/src/lib.rs`

**Interfaces:**
- Consumes: `SkillVersionV1`, `SkillPointerV1`, and their signed-event builders from Task 1.
- Produces: `SkillLearningRuntime::observe_turn(TurnLearningEvidence) -> LearningAction` where `LearningAction` is `None`, `Promoted { skill_id, version_id }`, or `RolledBack { skill_id, version_id }`.
- Produces: `SkillRegistry::ready_for_publish()` and idempotent state transitions `pending -> version_published -> pointer_published -> materialized`.

- [ ] **Step 1: Write failing state-machine tests**

  Prove that one success does nothing; two successes with the same normalized bounded task create one candidate; duplicate delivery is idempotent; different tasks do not combine; inherited tests are preserved; a removed inherited check rejects a candidate; forbidden permission/network/credential/release text rejects a candidate; a passing candidate produces version then pointer publication work; a failed candidate never changes the active pointer; two matching failures request rollback; and a crash at every outbox state resumes without duplicate version or pointer events.

- [ ] **Step 2: Run the focused test and confirm failure**

  ```bash
  . ./bin/activate-hermit && cargo test -p buzz-acp skill_learning
  ```

  Expected: compilation fails because the `skill_learning` module is absent.

- [ ] **Step 3: Implement normalized workflow detection and candidate generation**

  Lowercase the task, replace identifier-like and numeric runs, collapse whitespace, retain at most 512 bytes, and hash the normalized result. The managed skill id is `learned-<first 12 lowercase SHA-256 hex>`. Require two distinct successful experience IDs.

  Generate this bounded structure without an LLM:

  ```markdown
  ---
  name: learned-<hash>
  description: Reusable procedure learned from repeated successful Command Adviser work.
  ---
  # Repeated task pattern
  <redacted bounded normalized request>

  # Procedure
  1. Confirm the current request, due time, and missing inputs.
  2. Recall relevant active experience and retrieve applicable cited doctrine or reference evidence.
  3. Complete the work with the information available; identify missing facts and material risk without inventing status.
  4. Return the concise result, proposed action, and any follow-up needed.

  # Boundaries
  This skill does not grant tools, credentials, network access, provider changes, release changes, or external-action authority.
  ```

- [ ] **Step 4: Implement deterministic evaluation**

  Required checks are: valid frontmatter and exact name, body/hash/size, permitted scope, empty-or-allowlisted `required_tools`, every inherited check present byte-for-byte, trigger terms present, all source experience IDs distinct, and a case-insensitive prohibited-pattern scan for credentials, new endpoints, cloud fallback, security-policy changes, model installation, release configuration, and autonomous external action. The evaluator returns stable check IDs and cannot be overridden by skill text.

- [ ] **Step 5: Implement the SQLite registry and publication outbox**

  Use WAL plus `synchronous=FULL`. Store only bounded task patterns, hashes, signed event JSON, evaluation results, and state—never raw transcripts or credentials. Reusing an ID with divergent bytes is a conflict. Publication order is version first, then pointer; materialization cannot begin before both relay acknowledgements.

- [ ] **Step 6: Run the focused tests**

  ```bash
  . ./bin/activate-hermit && cargo test -p buzz-acp skill_learning
  ```

  Expected: learner, evaluation, lineage, rollback-threshold, and crash-recovery tests pass.

- [ ] **Step 7: Commit the learner**

  ```bash
  git add crates/buzz-acp/src/skill_learning crates/buzz-acp/src/skill_learning_tests.rs crates/buzz-acp/src/lib.rs
  git commit -s -m "feat: add bounded autonomous skill learner"
  ```

---

### Task 3: Materialize verified skills and rebuild disposable state

**Files:**
- Create: `crates/buzz-acp/src/skill_learning/materialize.rs`
- Create: `crates/buzz-acp/src/skill_learning/rebuild.rs`
- Modify: `crates/buzz-acp/src/skill_learning/mod.rs`
- Modify: `crates/buzz-acp/src/skill_learning_tests.rs`
- Modify: `crates/buzz-agent/src/hints.rs`

**Interfaces:**
- Consumes: relay-acknowledged `SkillVersionV1` and `SkillPointerV1` records from Tasks 1-2.
- Produces: `materialize_active_skills(root, active) -> MaterializeReport` and `rebuild_registry(rest, keys, owner, registry) -> RebuildReport`.

- [ ] **Step 1: Write failing materialization and rebuild tests**

  Cover atomic first install, atomic version replacement, stale managed directory removal, preservation of ordinary user skill directories, content-hash mismatch refusal, path traversal refusal, corrupt pointer isolation, missing-version pointer isolation, valid highest-head selection, deletion of SQLite followed by relay rebuild, byte-identical materialization after rebuild, and restart with the prior known-good projection when relay fetch is unavailable.

- [ ] **Step 2: Run focused tests and confirm failure**

  ```bash
  . ./bin/activate-hermit && cargo test -p buzz-acp skill_materialize skill_rebuild
  ```

  Expected: compilation fails because materialization and rebuild functions are absent.

- [ ] **Step 3: Implement atomic managed projection**

  Stage `SKILL.md` and `.skill-version.json` under a sibling temporary directory, fsync files and directory, verify the manifest hash, then rename into `.agents/skills/learned-<hash>`. Only directories matching the reserved prefix and carrying a valid managed marker may be replaced or removed. A failed activation leaves the previous directory unchanged.

- [ ] **Step 4: Implement authoritative relay rebuild**

  Query kinds `30180` and `30181` in dedicated filters scoped to the agent author and owner `#p`. Verify Nostr signatures before decrypting, validate each payload and hash, select NIP-33 heads for active pointers, reject pointers to absent or invalid versions, then replace the SQLite derived tables in one transaction and materialize the resulting active set.

- [ ] **Step 5: Harden `buzz-agent` discovery for managed markers**

  Ordinary skills keep current behaviour. For `learned-*`, require `.skill-version.json`, ensure its `skill_id` equals the directory/frontmatter name, and verify its recorded SHA-256 before advertising or loading the skill. A corrupt managed directory is skipped rather than treated as instructions.

- [ ] **Step 6: Run materializer, rebuild, and loader tests**

  ```bash
  . ./bin/activate-hermit && \
    cargo test -p buzz-acp skill_materialize && \
    cargo test -p buzz-acp skill_rebuild && \
    cargo test -p buzz-agent hints
  ```

  Expected: all tests pass, user skills survive, and corrupt managed projections are absent from discovery.

- [ ] **Step 7: Commit materialization and rebuild**

  ```bash
  git add crates/buzz-acp crates/buzz-agent
  git commit -s -m "feat: materialize and rebuild active agent skills"
  ```

---

### Task 4: Wire learning into real adviser turns and between-turn reload

**Files:**
- Modify: `crates/buzz-acp/src/experience_capture.rs`
- Modify: `crates/buzz-acp/src/experience_capture_tests.rs`
- Modify: `crates/buzz-acp/src/pool.rs`
- Modify: `crates/buzz-acp/src/lib.rs`
- Modify: `crates/buzz-acp/src/skill_learning/mod.rs`
- Modify: `crates/buzz-acp/src/skill_learning_tests.rs`

**Interfaces:**
- Consumes: `recall_query` already assembled by `run_prompt_task` and terminal `PromptOutcome`.
- Produces: actual `ExperienceRecordV1.task_summary`, `skill_versions`, and validation results for the frozen turn snapshot.
- Produces: a session invalidation signal after promotion or rollback so only the next turn loads the changed skill.

- [ ] **Step 1: Write failing integration tests**

  Prove that bounded user task text—not the placeholder event-count string—feeds experience and learning; secret-shaped text is redacted before SQLite or signed events; the active skill snapshot is frozen at `begin_turn`; a promotion during `finish_turn` invalidates the channel session only after the completed response; the next session discovers the new skill; a first matching failure records regression evidence without rollback; a second matching failure activates the parent; unrelated failures do not affect the skill; and failed learning never changes the adviser response outcome.

- [ ] **Step 2: Run focused tests and confirm failure**

  ```bash
  . ./bin/activate-hermit && cargo test -p buzz-acp experience_ skill_turn
  ```

  Expected: the new task-text and session-invalidation assertions fail.

- [ ] **Step 3: Pass bounded task text into experience capture**

  Add `task_text: &str` to `begin_turn`, store a redacted 4 KiB summary, and snapshot the active registry versions before prompt dispatch. Preserve source event IDs by reference. Do not capture the final hidden reasoning, provider payload, tool arguments, or raw transcript.

- [ ] **Step 4: Evaluate learning after terminal outcome**

  Make `finish_turn` return `LearningAction`. On success, record the observation, evaluate/publish/materialize an eligible candidate, and return `Promoted` only after the active pointer acknowledgement. On failure, increment only the matching active skill's regression counter and return `RolledBack` after two distinct failures and successful rollback-pointer acknowledgement. Registry or relay errors log `skill_learning_degraded` and return `None`.

- [ ] **Step 5: Reload only between turns**

  In `send_prompt_result`, invalidate the completed source session when `finish_turn` returns `Promoted` or `RolledBack`; do not kill the process and do not modify a running turn's skill vector. The following `session/new` performs the existing fresh filesystem discovery.

- [ ] **Step 6: Run the integration tests**

  ```bash
  . ./bin/activate-hermit && \
    cargo test -p buzz-acp experience_ && \
    cargo test -p buzz-acp skill_turn && \
    cargo test -p buzz-agent hints_integration
  ```

  Expected: all focused tests pass and a learning outage remains fail-soft for adviser work.

- [ ] **Step 7: Commit the runtime integration**

  ```bash
  git add crates/buzz-acp crates/buzz-agent
  git commit -s -m "feat: evolve skills between adviser turns"
  ```

---

### Task 5: Add operations, focused gates, and installed-app acceptance

**Files:**
- Create: `scripts/check-autonomous-skills.sh`
- Create: `docs/command-console/autonomous-skills-operations.md`
- Modify: `docs/command-console/ROADMAP.md`

**Interfaces:**
- Consumes: all Phase 4 contracts and runtime paths.
- Produces: one repeatable command for code gates and a live acceptance record suitable for the Phase 4 PR.

- [ ] **Step 1: Write the focused gate script**

  The script must run, in order:

  ```bash
  cargo test -p buzz-core agent_skill
  cargo test -p buzz-relay agent_skill
  cargo test -p buzz-search p_gated
  cargo test -p buzz-acp skill_
  cargo test -p buzz-acp experience_
  cargo test -p buzz-agent hints
  ```

  It exits non-zero on the first failure and prints `all autonomous-skill checks passed` only after every command succeeds.

- [ ] **Step 2: Document bounded operation and recovery**

  Record the event kinds, SQLite path, managed directory, two-success promotion threshold, two-failure rollback threshold, `skill_learning_degraded` meaning, rebuild source of truth, user-skill preservation rule, and rollback acceptance steps. State plainly that v1 generates a conservative text-only checklist and does not change permissions.

- [ ] **Step 3: Run focused and full quality gates**

  ```bash
  . ./bin/activate-hermit && bash scripts/check-autonomous-skills.sh
  . ./bin/activate-hermit && just ci
  ```

  Expected: both commands exit 0.

- [ ] **Step 4: Build and install the stable-signed application**

  ```bash
  . ./bin/activate-hermit && just desktop-release-build
  ```

  Verify Developer ID identity `SR52Q9EJ76`, retain the current application as a named rollback bundle, install the candidate, launch it, and verify one desktop process, nine `buzz-acp` processes, and one Apple-input watcher without another Keychain approval.

- [ ] **Step 5: Execute the real Phase 4 user journey**

  With one specialist, submit the same harmless bounded workflow twice. Verify: the first success leaves no candidate; the second creates, evaluates, publishes, materializes, and activates one `learned-*` skill without approval; the next turn's fresh session advertises that exact version. Then inject two deterministic matching failure outcomes in the acceptance harness and verify the pointer returns to the parent while both versions and all evaluation evidence remain queryable. Restart Command Adviser, remove the disposable SQLite/materialized copy in a backed-up test profile, rebuild from relay events, and verify byte-identical active content.

- [ ] **Step 6: Update the PR and Memory MCP checkpoint**

  Add exact commits, test counts, signed bundle identity, event IDs, active version, rollback version, rebuild hash, and remaining limitations to the draft PR. Record only the high-value architecture and live acceptance result in Memory MCP with agent `CODEX`.

- [ ] **Step 7: Commit the operations material**

  ```bash
  git add scripts/check-autonomous-skills.sh docs/command-console
  git commit -s -m "docs: add autonomous skill operations and acceptance"
  ```

---

## Self-Review

- Spec coverage: immutable versions, active pointer, private/shared scope, inherited tests, deterministic promotion, rollback, managed materialization, corrupt-cache rebuild, restart, and a real later-turn load are each assigned to an explicit task.
- Scope control: model-authored skill synthesis, supporting-file object storage, new UI panels, and external-action automation remain outside v1; the text-only deterministic learner is a usable first increment.
- Type consistency: `skill_id`, `version_id`, `parent_version_id`, `active_version_id`, `source_experience_ids`, and `evaluation_ids` retain the same names across core, registry, rebuild, and runtime integration.
- Placeholder scan: the plan contains no deferred implementation markers; each gate names its exact behaviour and command.
