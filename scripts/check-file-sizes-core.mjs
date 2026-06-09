import { promises as fs } from "node:fs";
import path from "node:path";

/**
 * Shared file-size check used by the desktop and web workspaces.
 *
 * Each app supplies its own `rules` (which roots/extensions to scan) and an
 * optional `overrides` map of TEMP per-file ceilings. Everything else — the
 * walk, the line count, the violation report, the non-zero exit — lives here so
 * the two apps can never drift.
 */

async function walkFiles(directory) {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  const files = await Promise.all(
    entries.map(async (entry) => {
      const fullPath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        return walkFiles(fullPath);
      }

      return [fullPath];
    }),
  );

  return files.flat();
}

function findRule(rules, relativePath) {
  return rules.find((rule) => {
    const normalizedRoot = `${rule.root}${path.sep}`;
    return relativePath.startsWith(normalizedRoot);
  });
}

function countLines(content) {
  if (content.length === 0) {
    return 0;
  }

  return content.split(/\r?\n/).length;
}

/**
 * @param {object} options
 * @param {string} options.projectRoot Absolute path the rule roots resolve against.
 * @param {Array<{root: string, extensions: Set<string>, maxLines: number}>} options.rules
 * @param {string} options.label Human label for the failure header (e.g. "Desktop").
 * @param {Map<string, number>} [options.overrides] TEMP per-file ceilings, keyed by path relative to projectRoot.
 * @param {string} options.scriptPath Path mentioned in the failure hint where overrides live.
 */
export async function runFileSizeCheck({
  projectRoot,
  rules,
  label,
  overrides = new Map(),
  scriptPath,
}) {
  const candidateFiles = (
    await Promise.all(
      rules.map((rule) => {
        const dir = path.join(projectRoot, rule.root);
        return fs
          .access(dir)
          .then(() => walkFiles(dir))
          .catch(() => []);
      }),
    )
  ).flat();

  const violations = [];

  for (const filePath of candidateFiles) {
    const relativePath = path.relative(projectRoot, filePath);
    const rule = findRule(rules, relativePath);
    if (!rule) {
      continue;
    }

    const extension = path.extname(relativePath);
    if (!rule.extensions.has(extension)) {
      continue;
    }

    const limit = overrides.get(relativePath) ?? rule.maxLines;
    const content = await fs.readFile(filePath, "utf8");
    const lineCount = countLines(content);
    if (lineCount > limit) {
      violations.push({ limit, lineCount, relativePath });
    }
  }

  if (violations.length > 0) {
    console.error(`${label} file size check failed:`);
    for (const violation of violations) {
      console.error(
        `- ${violation.relativePath}: ${violation.lineCount} lines (limit ${violation.limit})`,
      );
    }
    console.error(
      `Split the file or add a narrowly scoped exception in \`${scriptPath}\`.`,
    );
    process.exit(1);
  }
}
