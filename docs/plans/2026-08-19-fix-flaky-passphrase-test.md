---
title: Fix Flaky Passphrase Test - Plan
type: fix
date: 2026-08-19
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Fix Flaky Passphrase Test - Plan

## Goal Capsule

- **Objective:** Eliminate the flakiness in `generated_passphrase_respects_word_count_and_separator` so it passes deterministically every run.
- **Authority:** The settled decisions in this plan's KTDs are binding; implementation must not expand scope.
- **Stop conditions:** The test passes 100% of runs; no production code changed; the `-` separator stays in use per KTD1; CI stays green.
- **Execution profile:** Single-unit, test-only fix in one crate.
- **Tail ownership:** The implementing agent owns verification and commit; no cross-crate tail.

## Product Contract

### Summary

The desktop key-backup passphrase generator draws words from the EFF short wordlist 2.0, which contains the hyphenated word `yo-yo`. The test `generated_passphrase_respects_word_count_and_separator` joins three words with `-` and asserts that splitting on `-` yields exactly three parts. When `yo-yo` is drawn, the split yields four parts, so the assertion fails. This is a test defect, not a product defect: the passphrase generator itself is correct, and `yo-yo` is a legitimate wordlist word.

### Problem Frame

A randomized test asserts on a property that the generator does not guarantee. The generator guarantees a word count joined by a separator; it does not guarantee that the separator never appears inside a word. The test conflates "number of words" with "number of separator-delimited segments." The failure is intermittent and depends on the OS entropy draw, surfacing roughly 1 in 186 runs for the 3-word `-` case.

### Requirements

- R1. The test `generated_passphrase_respects_word_count_and_separator` must pass deterministically on every run, including runs where the wordlist word `yo-yo` is drawn.
- R2. The test must still verify that the generated phrase contains exactly the requested number of words, all drawn from the EFF short wordlist.
- R3. The fix must not alter the behavior of `generate_passphrase` in `desktop/src-tauri/src/key_backup.rs`; production code stays untouched unless a unit explicitly requires otherwise.
- R4. The fix must keep the existing length gate assertion (`phrase.chars().count() >= MIN_PASSPHRASE_LEN`).
- R5. The test must keep exercising the real `-` separator (the production path), per KTD1's settled rationale.

### Actors

- A1. The Rust test runner (`cargo test`).
- A2. The EFF short wordlist 2.0 (source of `yo-yo`).

### Acceptance Examples

- AE1. **Covers R1, R2.** When the word-aware split is fed a `-`-joined phrase in which `yo-yo` is one of the drawn words, the split never mis-counts: it reconstructs `yo-yo` as a single word and the phrase contains exactly `count` words. The deterministic fixture in TS1 guarantees this runs on every invocation, independent of the entropy draw.

### Scope Boundaries

- **In scope:** The single flaky test, and any helper the fix introduces for word-aware splitting.
- **Out of scope:** Changing the wordlist, changing `generate_passphrase` behavior, changing the separator for production passphrases, and fixing any other tests.

### Dependencies

- EFF short wordlist 2.0 at `desktop/src-tauri/src/assets/eff_short_wordlist_2_0.txt` (contains `yo-yo` at line 1281).

## Planning Contract

### Key Technical Decisions

- KTD1. Make the test separator-aware by splitting on the separator and then re-joining hyphenated-word fragments: split the phrase on the separator into fragments, then walk left-to-right and join adjacent fragments (with the separator) whenever the joined fragment is a wordlist word. **(session-settled: user-directed — chosen over changing the generator to reject hyphenated words: the generator is correct and the wordlist is authoritative.)**
  - Rationale: The generator's contract is word-count, not segment-count. The test should verify the contract. The sibling test `generated_passphrase_clamps_word_count` already uses `|` as a separator precisely because it cannot appear in the wordlist — but changing the separator alone would silently stop exercising the real `-` production path. The robust fix is to make the assertion word-aware. `str::split` removes the delimiter, so a "part still contains the separator" branch can never fire; re-joining fragments is the correct mechanism.
- KTD2. Introduce a small test-local helper that splits a phrase into words on the separator but does not split inside a known hyphenated word.
  - Rationale: The wordlist has exactly one hyphenated word (`yo-yo`). Splitting on the separator and then re-joining a `yo-yo` fragment is a 3-line, deterministic, no-dependency approach that keeps the test readable. The re-join is unambiguous because `yo` alone is not a wordlist word (verified: `^yo$` matches nothing in the wordlist).
- KTD3. Keep the fix test-only; do not modify `key_backup.rs` production code.
  - Rationale: There is no production defect. Modifying the generator to avoid `yo-yo` would degrade passphrase entropy for no user benefit.

### Assumptions

- The wordlist will not gain new hyphenated words in the near term. If it does, the helper's re-join logic still handles them, provided their segments are not themselves standalone wordlist words (the `yo-yo` case is unambiguous today because `yo` alone is not in the wordlist).

### Sequencing

- Single unit. No cross-unit ordering constraints.

## Implementation Units

### U1. Fix the flaky passphrase word-count test

- **Goal:** Make `generated_passphrase_respects_word_count_and_separator` deterministic and word-count-correct.
- **Requirements:** R1, R2, R3, R4, R5
- **Files:**
  - `desktop/src-tauri/src/key_backup_tests.rs` (test change)
- **Approach:**
  - In the test, replace the naive `phrase.split(separator).len()` assertion with a word-aware split that reconstructs hyphenated words. Concretely: split the phrase on the separator into fragments, then walk left-to-right, joining an adjacent fragment (with the separator) when the joined fragment is a wordlist word — the only current case being `yo-yo`. Assert the resulting word count equals `count`, and assert each reconstructed word is in the wordlist.
  - Add a comment noting the `yo-yo` case and why the split is word-aware (the re-join is unambiguous because `yo` alone is not a wordlist word).
  - Add a deterministic test helper or assertion that exercises the word-aware split with a literal `yo-yo` phrase (e.g., a phrase assembled from `yo-yo` plus two other wordlist words), so the special case runs on every test invocation regardless of the entropy draw.
  - Keep the `MIN_PASSPHRASE_LEN` gate assertion unchanged.
- **Patterns:** Follow the existing test style in the file: `WORDLIST.lines().filter(|l| !l.is_empty()).collect()` into a `HashSet`, and existing `assert_eq!`/`assert!` usage.
- **Test Scenarios:**
  - TS1. A deterministic fixture: a literal `-`-joined phrase containing `yo-yo` (e.g., `"yo-yo-ability-fire"`) fed to the word-aware split yields a word count of 3, with each reconstructed word in the wordlist. This runs on every invocation.
  - TS2. A phrase drawn from the wordlist with a `-` separator still yields a word count equal to `count` (covers both `yo-yo`-present and `yo-yo`-absent draws).
  - TS3. The existing loop tuple entries with separator `" "` (space) and `"."` continue to split correctly through the word-aware helper (these separators have no hyphenated-word interference).
  - TS4. The empty-separator member of the loop tuple (a single concatenated string) still reaches the shared length-gate assertion.
- **Verification:**
  - Run `cargo test --manifest-path desktop/src-tauri/Cargo.toml key_backup` (or the targeted test name) and confirm it passes.
  - Optionally run the test in a loop (e.g., `-- --test-threads=1` with many iterations) to confirm determinism.

## Verification Contract

- **Primary command:** `cargo test --manifest-path desktop/src-tauri/Cargo.toml key_backup`
- **Full crate test:** `cargo test --manifest-path desktop/src-tauri/Cargo.toml`
- **Quality gates:**
  - The targeted test passes.
  - No production source files changed (verify with `git status` / `git diff --stat`).
  - The existing sibling tests (`generated_passphrase_clamps_word_count`, `generated_passphrases_are_not_repeated`, NFKC round-trip) still pass.
  - No new warnings introduced.

## Definition of Done

### Global

- The flaky test is deterministic.
- All tests in the `key_backup` module pass.
- No production code changed.
- The diff is limited to the test file (plus the plan document under `docs/plans/`).

### Per-Unit

- U1. Done when the test file change is complete, the targeted test passes, the deterministic `yo-yo` fixture passes, and the full `key_backup` module passes.
- Cleanup: no dead code, no commented-out branches, no leftover scratch files in the diff.
