#!/usr/bin/env node
/**
 * Generate the exhaustive NIP-AD lifecycle conformance corpus.
 *
 * Enumerates every bound-disposition sequence up to length 4 (340 histories)
 * and computes each expected result from the declarative rules below — NOT by
 * calling either implementation. That independence is the point: a corpus
 * generated from one implementation only proves the other agrees with it,
 * which is exactly how a spec/implementation contradiction survived three
 * review rounds. Here the rules are stated a third time, in the plainest form
 * they have, and all three must agree.
 *
 * Also generates the normative transition table embedded in NIP-AD.md, so the
 * spec cannot drift from the behavior either.
 *
 * Usage: node scripts/gen-nip-ad-corpus.mjs [--check]
 *   --check exits nonzero if the committed files are stale (used by CI).
 */

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const CORPUS = join(ROOT, "docs/nips/nip-ad-lifecycle-exhaustive.json");

const STATES = ["completed", "refused", "responded", "errored"];
const TERMINAL = new Set(["completed", "refused"]);

// ---------------------------------------------------------------------------
// The rules, stated declaratively. This is the third independent statement of
// the lifecycle (Rust, TypeScript, here) and the one the other two are checked
// against.
// ---------------------------------------------------------------------------

/** Terminal-absorbing outcome. Order is the bound, sorted history. */
function expectedOutcome(history) {
  const hasCompleted = history.includes("completed");
  const hasRefused = history.includes("refused");
  const firstTerminal = history.findIndex((s) => TERMINAL.has(s));

  // Both terminal types claimed: no settled answer exists.
  if (hasCompleted && hasRefused) {
    return { kind: "disputed" };
  }
  // A terminal claim absorbs everything ordered after it.
  if (firstTerminal !== -1) {
    return { kind: "settled", state: history[firstTerminal] };
  }
  // Nothing terminal: open if anything bound at all, else a real gap.
  if (history.length > 0) {
    return { kind: "open", state: history[history.length - 1] };
  }
  return { kind: "unanswered" };
}

/**
 * Diagnostics that do not change the outcome. Deliberately non-overlapping:
 * a terminal following a terminal is already fully described by
 * `duplicate_terminal` (same state) or by a disputed outcome (opposing
 * states), so `ordered_after_terminal` is scoped to non-terminal followers.
 */
function expectedWarnings(history) {
  const warnings = [];
  // "Duplicate" means the SAME terminal state twice. Counting any two
  // terminals would fire on `completed → refused`, which is a contradiction
  // (already `disputed`), not a duplicate.
  const duplicated = [...TERMINAL].some(
    (state) => history.filter((s) => s === state).length > 1,
  );
  if (duplicated) {
    warnings.push("duplicate_terminal");
  }
  const firstTerminal = history.findIndex((s) => TERMINAL.has(s));
  if (
    firstTerminal !== -1 &&
    history.slice(firstTerminal + 1).some((s) => !TERMINAL.has(s))
  ) {
    warnings.push("ordered_after_terminal");
  }
  return warnings;
}

/** The last event's state, regardless of absorption. */
function expectedLatestObservation(history) {
  return history.length > 0 ? history[history.length - 1] : null;
}

/** Settled histories are resolved. Disputed ones are not. */
function expectedResolved(history) {
  return expectedOutcome(history).kind === "settled";
}

// ---------------------------------------------------------------------------
// Enumeration
// ---------------------------------------------------------------------------

function sequencesUpTo(maxLen) {
  const out = [[]];
  let frontier = [[]];
  for (let len = 1; len <= maxLen; len += 1) {
    const next = [];
    for (const seq of frontier) {
      for (const state of STATES) {
        next.push([...seq, state]);
      }
    }
    out.push(...next);
    frontier = next;
  }
  return out;
}

function buildCorpus() {
  const cases = sequencesUpTo(4).map((history) => ({
    history,
    expect: {
      outcome: expectedOutcome(history),
      latestObservation: expectedLatestObservation(history),
      warnings: expectedWarnings(history),
      resolved: expectedResolved(history),
    },
  }));

  return {
    $comment: [
      "GENERATED FILE -- do not edit by hand.",
      "Regenerate with: node scripts/gen-nip-ad-corpus.mjs",
      "",
      "Every bound-disposition history up to length 4, with expectations",
      "computed from declarative rules in the generator rather than from",
      "either implementation. Both the Rust verifier and the TypeScript",
      "mirror run all of these.",
      "",
      "Histories are already ordered: the runner assigns increasing",
      "created_at values, so index order is sort order. Ordering itself is",
      "covered by the hand-written cases in nip-ad-conformance.json.",
    ],
    caseCount: cases.length,
    cases,
  };
}

// ---------------------------------------------------------------------------
// The normative transition table in NIP-AD.md, generated from the same rules.
// ---------------------------------------------------------------------------

function describe(history) {
  return history.length === 0 ? "(nothing bound)" : history.join(" → ");
}

function renderOutcome(outcome) {
  switch (outcome.kind) {
    case "settled":
      return `**settled** (\`${outcome.state}\`)`;
    case "open":
      return `open (\`${outcome.state}\`)`;
    case "disputed":
      return "**disputed**";
    default:
      return "unanswered";
  }
}

/**
 * The representative histories the spec documents. Chosen to cover every
 * distinct (outcome kind, warning set) pair the rules can produce, so the
 * table is complete in behavior even though it is not the full 340.
 */
function tableRows() {
  const seen = new Set();
  const rows = [];
  for (const history of sequencesUpTo(3)) {
    const outcome = expectedOutcome(history);
    const warnings = expectedWarnings(history);
    const key = `${outcome.kind}:${outcome.state ?? ""}:${warnings.join(",")}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    rows.push({ history, outcome, warnings });
  }
  return rows;
}

function renderTable() {
  const lines = [
    "| History (ordered) | Effective outcome | Resolved? | Warnings |",
    "|---|---|---|---|",
  ];
  for (const { history, outcome, warnings } of tableRows()) {
    lines.push(
      `| ${describe(history)} | ${renderOutcome(outcome)} | ${
        outcome.kind === "settled" ? "yes" : "no"
      } | ${warnings.length ? warnings.map((w) => `\`${w}\``).join(", ") : "—"} |`,
    );
  }
  return lines.join("\n");
}

const NIP = join(ROOT, "docs/nips/NIP-AD.md");
const BEGIN = "<!-- BEGIN GENERATED: transition-table -->";
const END = "<!-- END GENERATED: transition-table -->";

function spliceTable(markdown, table) {
  const start = markdown.indexOf(BEGIN);
  const end = markdown.indexOf(END);
  if (start === -1 || end === -1) {
    throw new Error(
      `NIP-AD.md is missing the ${BEGIN} / ${END} markers — the generated ` +
        "transition table has nowhere to go.",
    );
  }
  return `${markdown.slice(0, start + BEGIN.length)}\n${table}\n${markdown.slice(end)}`;
}

// ---------------------------------------------------------------------------

const check = process.argv.includes("--check");
const corpus = `${JSON.stringify(buildCorpus(), null, 2)}\n`;
const nipBefore = readFileSync(NIP, "utf8");
const nipAfter = spliceTable(nipBefore, renderTable());

if (check) {
  const stale = [];
  if (readFileSync(CORPUS, "utf8") !== corpus) {
    stale.push("docs/nips/nip-ad-lifecycle-exhaustive.json");
  }
  if (nipBefore !== nipAfter) {
    stale.push("docs/nips/NIP-AD.md (transition table)");
  }
  if (stale.length > 0) {
    console.error(
      `Stale generated NIP-AD artifacts:\n  ${stale.join("\n  ")}\n\n` +
        "The spec's transition table and the exhaustive corpus are generated " +
        "from one rule set so they cannot drift from each other.\n" +
        "Run: node scripts/gen-nip-ad-corpus.mjs",
    );
    process.exit(1);
  }
  console.log("NIP-AD generated artifacts are up to date.");
} else {
  writeFileSync(CORPUS, corpus);
  writeFileSync(NIP, nipAfter);
  console.log(
    `Wrote ${JSON.parse(corpus).caseCount} lifecycle cases and the NIP-AD transition table.`,
  );
}
