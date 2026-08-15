/**
 * Rank existing channel files as candidates for "this upload is a new version
 * of that file".
 *
 * Pure and dependency-free, in a `.mjs` sibling so `node:test` can exercise
 * the exact source the composer runs (same rationale as
 * `applyEditTagOverlay.mjs`). The matching heuristics are the part most likely
 * to need tuning against real filenames, so they need to be testable without
 * mounting a React tree.
 *
 * Why fuzzy at all: the original composer check required a byte-identical
 * filename, which only fires when someone re-exports under the exact same
 * name. Every ordinary versioning habit — `report-v2.pdf`, `report (1).pdf`,
 * `deck FINAL.pptx` — missed entirely. Fuzzy matching is safe here *because*
 * the composer now prompts on every upload: a weak guess costs a glance at a
 * dialog that was going to open anyway, and nothing is ever linked without an
 * explicit confirmation. Ranking, not deciding.
 */

/**
 * Candidates older than this are not considered for automatic matching.
 *
 * A new version of a file rarely lands more than a couple of months after the
 * one it replaces, and stale candidates are exactly where filename collisions
 * turn into bad guesses (last year's `notes.md` is not the parent of today's).
 * This bounds *suggestions* only — the picker still lists the channel's full
 * history, so linking to something older stays one scroll away.
 */
export const SUPERSEDES_MATCH_WINDOW_SECONDS = 62 * 24 * 60 * 60;

/** At or above this score a candidate is good enough to pre-select. */
export const SUPERSEDES_PRESELECT_SCORE = 75;

const EXACT_SCORE = 100;
const NORMALIZED_EQUAL_SCORE = 90;
const PREFIX_SCORE = 75;
const MAX_TOKEN_SCORE = 60;

/** Shortest stem that may be treated as a meaningful prefix match. */
const MIN_PREFIX_STEM_LENGTH = 3;

/** Split a filename into `[stem, extension]`, both lowercased. */
function splitFilename(filename) {
  const trimmed = String(filename ?? "").trim();
  const dot = trimmed.lastIndexOf(".");
  if (dot <= 0 || dot === trimmed.length - 1) {
    return [trimmed.toLowerCase(), ""];
  }
  return [
    trimmed.slice(0, dot).toLowerCase(),
    trimmed.slice(dot + 1).toLowerCase(),
  ];
}

/**
 * Strip the noise that distinguishes versions of the same document, so
 * `Report v2 (1) FINAL` and `report` reduce to the same stem.
 *
 * Order matters: separators are unified first so the later word-boundary
 * patterns match regardless of whether the author used `-`, `_`, `.` or spaces.
 */
export function normalizeStem(stem) {
  return (
    String(stem ?? "")
      .toLowerCase()
      .replace(/[._\-\s]+/g, " ")
      // v2, v 2, ver3, version 1.2, rev2, r4
      .replace(/\b(?:v|ver|version|rev|r)\s*\d+(?:\.\d+)*\b/g, " ")
      // trailing duplicate markers: (1), [2]
      .replace(/[([]\s*\d+\s*[)\]]/g, " ")
      // ISO-ish and compact dates: 2026-08-15, 2026_08_15, 20260815
      .replace(/\b\d{4}\s?\d{2}\s?\d{2}\b/g, " ")
      .replace(/\b\d{8}\b/g, " ")
      // editorial qualifiers people append instead of a version number
      .replace(/\b(?:copy|final|draft|latest|new|old|updated|update)\b/g, " ")
      // a bare trailing number is almost always a version counter
      .replace(/\s+\d+\s*$/, " ")
      .replace(/\s+/g, " ")
      .trim()
  );
}

/** Word tokens of a normalized stem, empties removed. */
function tokens(normalized) {
  return normalized.split(" ").filter(Boolean);
}

/** Jaccard overlap of two token sets, 0..1. */
function tokenOverlap(a, b) {
  if (a.length === 0 || b.length === 0) return 0;
  const setA = new Set(a);
  const setB = new Set(b);
  let shared = 0;
  for (const token of setA) {
    if (setB.has(token)) shared += 1;
  }
  const union = new Set([...setA, ...setB]).size;
  return union === 0 ? 0 : shared / union;
}

/**
 * Similarity of an upload's filename to an existing file's, 0..100.
 *
 * Exported for tests and for callers that want to explain a ranking; the
 * composer itself only needs `rankSupersedesCandidates`.
 */
export function scoreFilenameSimilarity(uploadFilename, candidateFilename) {
  const [uploadStem, uploadExt] = splitFilename(uploadFilename);
  const [candidateStem, candidateExt] = splitFilename(candidateFilename);

  if (!uploadStem || !candidateStem) return 0;
  // A different extension means a different artifact, not a new version of the
  // same one. Treated as a hard exclusion rather than a score penalty: a PDF
  // export of a DOCX is a genuinely different file, and conflating the two is
  // the most confusing wrong answer this can give.
  if (uploadExt !== candidateExt) return 0;

  if (uploadStem === candidateStem) return EXACT_SCORE;

  const normalizedUpload = normalizeStem(uploadStem);
  const normalizedCandidate = normalizeStem(candidateStem);
  if (!normalizedUpload || !normalizedCandidate) return 0;

  if (normalizedUpload === normalizedCandidate) return NORMALIZED_EQUAL_SCORE;

  const shorter =
    normalizedUpload.length <= normalizedCandidate.length
      ? normalizedUpload
      : normalizedCandidate;
  const longer =
    shorter === normalizedUpload ? normalizedCandidate : normalizedUpload;
  if (shorter.length >= MIN_PREFIX_STEM_LENGTH && longer.startsWith(shorter)) {
    return PREFIX_SCORE;
  }

  const overlap = tokenOverlap(
    tokens(normalizedUpload),
    tokens(normalizedCandidate),
  );
  return Math.round(overlap * MAX_TOKEN_SCORE);
}

/**
 * Rank `files` as version-parent candidates for an upload.
 *
 * Excluded outright: the upload itself (same content hash), files already
 * superseded by something else, files with no filename, files whose extension
 * differs, and anything outside the match window. Everything surviving is
 * returned newest-first within score order, so an equally-good recent file
 * wins over an equally-good old one.
 *
 * `nowSeconds` is injectable so the window is testable without freezing time.
 */
export function rankSupersedesCandidates(
  uploadFilename,
  uploadSha256,
  files,
  nowSeconds = Date.now() / 1000,
) {
  if (!uploadFilename) return [];
  const cutoff = nowSeconds - SUPERSEDES_MATCH_WINDOW_SECONDS;

  const scored = [];
  for (const file of files ?? []) {
    if (!file || !file.filename) continue;
    if (file.supersededBy != null) continue;
    if (uploadSha256 && file.sha256 === uploadSha256) continue;
    if (typeof file.uploadedAt === "number" && file.uploadedAt < cutoff)
      continue;

    const score = scoreFilenameSimilarity(uploadFilename, file.filename);
    if (score <= 0) continue;
    scored.push({ file, score });
  }

  scored.sort((a, b) => {
    if (b.score !== a.score) return b.score - a.score;
    return (b.file.uploadedAt ?? 0) - (a.file.uploadedAt ?? 0);
  });
  return scored;
}

/**
 * The single candidate worth pre-selecting, or null.
 *
 * Deliberately refuses to pre-select on a tie: two candidates scoring equally
 * well means the filename genuinely does not identify which one is the parent,
 * and silently picking the more recent would be a guess presented as an answer.
 * The prompt still opens with both ranked at the top — the user just has to say
 * which.
 */
export function preselectedSupersedesCandidate(ranked) {
  const [best, runnerUp] = ranked;
  if (!best || best.score < SUPERSEDES_PRESELECT_SCORE) return null;
  if (runnerUp && runnerUp.score === best.score) return null;
  return best.file;
}
