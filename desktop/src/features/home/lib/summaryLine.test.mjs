import assert from "node:assert/strict";
import test from "node:test";

import { summaryLineFor } from "./summaryLine.ts";

test("bold TL;DR marker returns the remainder of that line", () => {
  assert.equal(
    summaryLineFor(
      "**TL;DR** Relay migration is complete and green.\n\nLonger detail follows across several paragraphs.",
    ),
    "Relay migration is complete and green.",
  );
  assert.equal(
    summaryLineFor("**TL;DR:** Ship it tonight.\nDetail."),
    "Ship it tonight.",
  );
});

test("plain TLDR marker works, case-insensitively", () => {
  assert.equal(
    summaryLineFor("tldr: the backup ran clean.\nEverything else is noise."),
    "the backup ran clean.",
  );
  assert.equal(
    summaryLineFor("TL;DR the launch moved to Thursday."),
    "the launch moved to Thursday.",
  );
});

test("blockquoted TL;DR marker is recognised", () => {
  assert.equal(
    summaryLineFor("> **TL;DR** we are frozen until Monday.\n\nContext…"),
    "we are frozen until Monday.",
  );
});

test("declared-ask tier beats a TL;DR line", () => {
  assert.equal(
    summaryLineFor(
      [
        "**TL;DR** long status here.",
        "**Needs Lee, decision:** Ship now or wait for QA?",
      ].join("\n"),
    ),
    "Ship now or wait for QA?",
  );
});

test("no marker degrades to the first sentence of the first line", () => {
  assert.equal(
    summaryLineFor(
      "Engineering shipped the desktop build. More detail in thread.",
    ),
    "Engineering shipped the desktop build.",
  );
  // No sentence terminator: the whole first line, markdown stripped.
  assert.equal(
    summaryLineFor("**Status update** for the launch\nSecond line here"),
    "Status update for the launch",
  );
});

test("a code-block-first message skips the block", () => {
  assert.equal(
    summaryLineFor(
      '```json\n{"a": 1}\n```\nDeploy config updated for staging.',
    ),
    "Deploy config updated for staging.",
  );
});

test("empty or code-only bodies return the empty string", () => {
  assert.equal(summaryLineFor(""), "");
  assert.equal(summaryLineFor("   \n\n  "), "");
  assert.equal(summaryLineFor("```\nonly code\n```"), "");
});

test("long summaries truncate on a word boundary", () => {
  const long = `The migration finished ${"and the relay stayed healthy ".repeat(8)}end`;
  const line = summaryLineFor(long);
  assert.ok(line.length <= 121);
  assert.ok(line.endsWith("…"));
});
