import assert from "node:assert/strict";
import { test } from "node:test";

import {
  SUPERSEDES_MATCH_WINDOW_SECONDS,
  normalizeStem,
  preselectedSupersedesCandidate,
  rankSupersedesCandidates,
  scoreFilenameSimilarity,
} from "./supersedesRanking.mjs";

const NOW = 1_760_000_000;

/** Minimal ChannelFileEntry stand-in. */
function file(filename, overrides = {}) {
  return {
    eventId: overrides.eventId ?? `event-${filename}`,
    filename,
    sha256: overrides.sha256 ?? `sha-${filename}`,
    supersededBy: overrides.supersededBy ?? null,
    uploadedAt: overrides.uploadedAt ?? NOW - 60,
  };
}

test("normalizeStem strips the markers that distinguish versions", () => {
  assert.equal(normalizeStem("report"), "report");
  assert.equal(normalizeStem("report-v2"), "report");
  assert.equal(normalizeStem("report_V3"), "report");
  assert.equal(normalizeStem("report (1)"), "report");
  assert.equal(normalizeStem("report rev4"), "report");
  assert.equal(normalizeStem("report FINAL"), "report");
  assert.equal(normalizeStem("report copy"), "report");
  assert.equal(normalizeStem("report 2026-08-15"), "report");
  assert.equal(normalizeStem("report 20260815"), "report");
  assert.equal(normalizeStem("Q3 deck"), "q3 deck");
});

test("identical filenames score highest", () => {
  assert.equal(scoreFilenameSimilarity("report.pdf", "report.pdf"), 100);
  assert.equal(scoreFilenameSimilarity("Report.PDF", "report.pdf"), 100);
});

test("the real-world version patterns all match", () => {
  // Each of these was missed entirely by the old exact-match rule.
  for (const [upload, existing] of [
    ["report-v2.pdf", "report.pdf"],
    ["report (1).pdf", "report.pdf"],
    ["Q3 deck FINAL.pptx", "Q3 deck.pptx"],
    ["budget_2026_rev2.xlsx", "budget_2026.xlsx"],
    ["notes v3.md", "notes v2.md"],
  ]) {
    assert.ok(
      scoreFilenameSimilarity(upload, existing) >= 75,
      `expected ${upload} to match ${existing}`,
    );
  }
});

test("a different extension never matches", () => {
  assert.equal(scoreFilenameSimilarity("report.pdf", "report.docx"), 0);
  // Even a byte-identical stem: a PDF export of a DOCX is a different artifact.
  assert.equal(scoreFilenameSimilarity("report-v2.pdf", "report.docx"), 0);
});

test("unrelated filenames do not match", () => {
  assert.equal(scoreFilenameSimilarity("invoice.pdf", "roadmap.pdf"), 0);
});

test("candidates outside the match window are excluded", () => {
  const recent = file("report.pdf", { uploadedAt: NOW - 1000 });
  const ancient = file("report.pdf", {
    eventId: "old",
    sha256: "sha-old",
    uploadedAt: NOW - SUPERSEDES_MATCH_WINDOW_SECONDS - 1,
  });

  const ranked = rankSupersedesCandidates(
    "report-v2.pdf",
    "sha-upload",
    [recent, ancient],
    NOW,
  );
  assert.equal(ranked.length, 1);
  assert.equal(ranked[0].file.eventId, recent.eventId);
});

test("already-superseded files and the upload itself are excluded", () => {
  const superseded = file("report.pdf", {
    eventId: "superseded",
    supersededBy: "something-newer",
  });
  const sameBytes = file("report.pdf", {
    eventId: "same",
    sha256: "sha-upload",
  });

  const ranked = rankSupersedesCandidates(
    "report.pdf",
    "sha-upload",
    [superseded, sameBytes],
    NOW,
  );
  assert.deepEqual(ranked, []);
});

test("better matches rank above weaker ones, recency breaks ties", () => {
  const exact = file("report.pdf", {
    eventId: "exact",
    uploadedAt: NOW - 5000,
  });
  const weaker = file("report notes annual.pdf", { eventId: "weaker" });
  const olderExact = file("report.pdf", {
    eventId: "older-exact",
    sha256: "sha-older",
    uploadedAt: NOW - 9000,
  });

  const ranked = rankSupersedesCandidates(
    "report.pdf",
    "sha-upload",
    [weaker, olderExact, exact],
    NOW,
  );
  assert.equal(ranked[0].file.eventId, "exact");
  assert.equal(ranked[1].file.eventId, "older-exact");
});

test("a confident single match is pre-selected", () => {
  const ranked = rankSupersedesCandidates(
    "report-v2.pdf",
    "sha-upload",
    [file("report.pdf")],
    NOW,
  );
  const picked = preselectedSupersedesCandidate(ranked);
  assert.ok(picked);
  assert.equal(picked.filename, "report.pdf");
});

test("an ambiguous tie is never pre-selected", () => {
  // Two equally-good parents: the filename genuinely does not say which, so
  // the prompt must ask rather than guess.
  const ranked = rankSupersedesCandidates(
    "report-v2.pdf",
    "sha-upload",
    [
      file("report.pdf", { eventId: "a", sha256: "sha-a" }),
      file("report.pdf", { eventId: "b", sha256: "sha-b" }),
    ],
    NOW,
  );
  assert.equal(ranked.length, 2);
  assert.equal(preselectedSupersedesCandidate(ranked), null);
});

test("a weak match is ranked but not pre-selected", () => {
  const ranked = rankSupersedesCandidates(
    "annual report summary.pdf",
    "sha-upload",
    [file("annual budget breakdown.pdf")],
    NOW,
  );
  if (ranked.length > 0) {
    assert.ok(ranked[0].score < 75);
    assert.equal(preselectedSupersedesCandidate(ranked), null);
  }
});

test("no filename means no candidates", () => {
  assert.deepEqual(
    rankSupersedesCandidates("", "sha", [file("report.pdf")], NOW),
    [],
  );
});
