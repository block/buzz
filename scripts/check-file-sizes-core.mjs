import { execFileSync } from "node:child_process";
import { promises as fs } from "node:fs";
import path from "node:path";

function git(args, cwd, options = {}) {
  return execFileSync("git", args, {
    cwd,
    encoding: "utf8",
    maxBuffer: 10 * 1024 * 1024,
    ...options,
  });
}

function toPosixPath(relativePath) {
  return relativePath.split(path.sep).join("/");
}

export function countLines(content) {
  if (content.length === 0) {
    return 0;
  }
  return content.split(/\r?\n/).length;
}

export function allowedLineCount(baseLines, maxLines) {
  return baseLines == null || baseLines <= maxLines ? maxLines : baseLines;
}

export function allowedLineCountFromBases(
  baseLineCounts,
  maxLines,
  overrideLines,
  commonBaseLines,
) {
  const inheritedLimit = baseLineCounts.reduce(
    (limit, baseLines) => Math.max(limit, allowedLineCount(baseLines, maxLines)),
    maxLines,
  );
  const combinedParentLimit =
    commonBaseLines == null || baseLineCounts.length < 2
      ? maxLines
      : commonBaseLines +
        baseLineCounts.reduce(
          (growth, baseLines) =>
            growth + Math.max(0, (baseLines ?? commonBaseLines) - commonBaseLines),
          0,
        );
  return Math.max(
    inheritedLimit,
    combinedParentLimit,
    overrideLines ?? maxLines,
  );
}

export function evaluateFileSize({ baseLines, candidateLines, maxLines }) {
  const limit = allowedLineCount(baseLines, maxLines);
  return { limit, violates: candidateLines > limit };
}

function findRule(rules, relativePath) {
  return rules.find((rule) => relativePath.startsWith(`${rule.root}/`));
}

export function resolveBaseRef(repoRoot, env = process.env) {
  if (env.CHECK_FILE_SIZES_BASE) {
    return env.CHECK_FILE_SIZES_BASE;
  }

  if (env.GITHUB_ACTIONS === "true") {
    return "HEAD^1";
  }

  try {
    const mergeBase = git(
      ["merge-base", "origin/main", "HEAD"],
      repoRoot,
    ).trim();
    const head = git(["rev-parse", "HEAD"], repoRoot).trim();
    return mergeBase === head ? "HEAD" : mergeBase;
  } catch (error) {
    throw new Error(
      "Could not resolve the file-size base from origin/main. Fetch origin/main or set CHECK_FILE_SIZES_BASE to an explicit commit.",
      { cause: error },
    );
  }
}

export function resolveBaseRefs(repoRoot, env = process.env) {
  if (env.CHECK_FILE_SIZES_BASE) {
    return [env.CHECK_FILE_SIZES_BASE];
  }

  let mergeHead = "";
  try {
    mergeHead = git(
      ["rev-parse", "--verify", "-q", "MERGE_HEAD"],
      repoRoot,
      {
        stdio: ["ignore", "pipe", "ignore"],
      },
    ).trim();
  } catch {
    // MERGE_HEAD exists only while a merge is in progress.
  }
  if (mergeHead) {
    return [git(["rev-parse", "HEAD"], repoRoot).trim(), mergeHead];
  }

  const [, ...parents] = git(
    ["rev-list", "--parents", "-n", "1", "HEAD"],
    repoRoot,
  )
    .trim()
    .split(/\s+/);
  if (parents.length > 1) {
    return parents;
  }

  if (env.GITHUB_ACTIONS === "true") {
    return parents.length > 0 ? parents : ["HEAD^1"];
  }

  return [resolveBaseRef(repoRoot, env)];
}

export function parseChangedFiles(output) {
  const fields = output.split("\0");
  const changes = [];

  for (let index = 0; index < fields.length - 1; ) {
    const status = fields[index++];
    if (status.startsWith("R") || status.startsWith("C")) {
      changes.push({
        status: status[0],
        oldPath: fields[index++],
        path: fields[index++],
      });
    } else {
      changes.push({ status: status[0], path: fields[index++] });
    }
  }

  return changes;
}

function changedProjectFiles({ repoRoot, projectRelative, baseRef }) {
  const output = git(
    ["diff", "--name-status", "-z", "-M", baseRef, "--", projectRelative],
    repoRoot,
  );
  const changes = parseChangedFiles(output);
  const trackedPaths = new Set(changes.map((change) => change.path));
  const untracked = git(
    ["ls-files", "--others", "--exclude-standard", "-z", "--", projectRelative],
    repoRoot,
  )
    .split("\0")
    .filter(Boolean);

  for (const filePath of untracked) {
    if (!trackedPaths.has(filePath)) {
      changes.push({ status: "A", path: filePath });
    }
  }
  return changes;
}

function readBaseFile(repoRoot, baseRef, filePath) {
  try {
    return git(["show", `${baseRef}:${filePath}`], repoRoot, {
      encoding: null,
      stdio: ["ignore", "pipe", "ignore"],
    }).toString("utf8");
  } catch {
    return null;
  }
}

export async function runFileSizeCheck({
  projectRoot,
  rules,
  label,
  overrides = new Map(),
}) {
  // Every governed project is a direct child of the repository root. Derive
  // these paths without Git so hook-provided repository environment variables
  // cannot collapse the project pathspec to an empty string.
  const repoRoot = path.dirname(projectRoot);
  const projectRelative = toPosixPath(path.basename(projectRoot));
  const baseRefs = resolveBaseRefs(repoRoot);

  // Fail clearly instead of silently turning a missing/shallow base into a pass.
  for (const baseRef of baseRefs) {
    git(["cat-file", "-e", `${baseRef}^{commit}`], repoRoot);
  }
  let commonBaseRef = null;
  if (baseRefs.length > 1) {
    try {
      commonBaseRef = git(["merge-base", ...baseRefs], repoRoot, {
        stdio: ["ignore", "pipe", "ignore"],
      }).trim();
    } catch {
      // A shallow CI checkout may contain both merge parents without their
      // common ancestor. Parent limits still enforce the ratchet; only the
      // optional allowance for independently combined growth is unavailable.
    }
  }

  const violations = [];
  const changesByPath = new Map();
  for (const baseRef of baseRefs) {
    for (const change of changedProjectFiles({
      repoRoot,
      projectRelative,
      baseRef,
    })) {
      const entry =
        changesByPath.get(change.path) ?? { path: change.path, bases: new Map() };
      entry.bases.set(baseRef, change);
      changesByPath.set(change.path, entry);
    }
  }

  for (const entry of changesByPath.values()) {
    const change = [...entry.bases.values()][0];
    if (change.status === "D") continue;

    const relativePath = toPosixPath(
      path.relative(projectRelative, change.path),
    );
    const rule = findRule(rules, relativePath);
    if (!rule || !rule.extensions.has(path.extname(relativePath))) continue;

    const candidatePath = path.join(repoRoot, change.path);
    const candidateLines = countLines(await fs.readFile(candidatePath, "utf8"));
    const baseLineCounts = baseRefs.map((baseRef) => {
      const baseChange = entry.bases.get(baseRef);
      const basePath = baseChange?.oldPath ?? change.path;
      const baseContent = readBaseFile(repoRoot, baseRef, basePath);
      return baseContent == null ? null : countLines(baseContent);
    });
    const commonBaseContent =
      commonBaseRef == null
        ? null
        : readBaseFile(repoRoot, commonBaseRef, change.path);
    const commonBaseLines =
      commonBaseContent == null ? null : countLines(commonBaseContent);
    const limit = allowedLineCountFromBases(
      baseLineCounts,
      rule.maxLines,
      overrides.get(relativePath),
      commonBaseLines,
    );
    const violates = candidateLines > limit;

    if (violates) {
      violations.push({
        relativePath,
        baseLineCounts,
        candidateLines,
        limit,
      });
    }
  }

  if (violations.length === 0) return;

  console.error(
    `${label} file size ratchet failed (base ${baseRefs.join(", ")}):`,
  );
  for (const violation of violations) {
    const inheritedLines = violation.baseLineCounts.filter(
      (baseLines) => baseLines != null,
    );
    const before =
      inheritedLines.length === 0 ? "new" : Math.max(...inheritedLines);
    const delta =
      before === "new"
        ? ""
        : ` (${violation.candidateLines - before >= 0 ? "+" : ""}${violation.candidateLines - before})`;
    console.error(
      `- ${violation.relativePath}: ${before} -> ${violation.candidateLines}${delta} lines (allowed ${violation.limit})`,
    );
  }
  console.error(
    "Keep new files at or below the limit; files already over it may not grow.",
  );
  process.exitCode = 1;
}
