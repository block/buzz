import type { AskType } from "@/features/attention/lib/taskExtraction";

/**
 * Tier-1 declared asks, parsed from the authoring convention documented in
 * crates/buzz-acp/src/base_prompt.md ("Declare what you need"):
 *
 *   **Needs <Name>, <type>:** <the ask in one sentence>
 *   - Option one as a complete sentence.
 *   - Option two as a complete sentence.
 *
 * One line per person and per ask; an optional bulleted list of two to four
 * closed-set options follows immediately (no blank line). Declared asks
 * always beat the derived classification tier.
 */

export type DeclaredAskType = Exclude<AskType, "headsUp">;

export type DeclaredAsk = {
  /** Addressee exactly as written between "Needs " and the comma. */
  name: string;
  type: DeclaredAskType;
  ask: string;
  /** Adopted closed-set answers; [] when absent, detached, or malformed. */
  options: string[];
};

const DECLARED_TYPES: ReadonlySet<string> = new Set([
  "decision",
  "approval",
  "question",
  "review",
  "blocked",
]);

const MIN_OPTIONS = 2;
const MAX_OPTIONS = 4;

const BOLD_DECLARATION = /^\*\*needs\s+([^,]+),\s*([a-z]+)\s*:\*\*\s*(.+)$/i;
const PLAIN_DECLARATION = /^needs\s+([^,]+),\s*([a-z]+)\s*:\s*(.+)$/i;
const BULLET = /^-\s+(.+)$/;

function parseDeclarationLine(
  line: string,
): Pick<DeclaredAsk, "name" | "type" | "ask"> | null {
  const trimmed = line.trim();
  const match =
    trimmed.match(BOLD_DECLARATION) ?? trimmed.match(PLAIN_DECLARATION);
  if (!match) {
    return null;
  }
  const type = match[2].toLowerCase();
  if (!DECLARED_TYPES.has(type)) {
    return null;
  }
  const name = match[1].trim();
  const ask = match[3].trim();
  if (!name || !ask) {
    return null;
  }
  return { name, type: type as DeclaredAskType, ask };
}

/** Consecutive "- " bullet lines starting at `start` (no blank line gap). */
function collectBullets(lines: string[], start: number): string[] {
  const bullets: string[] = [];
  for (let index = start; index < lines.length; index++) {
    const match = lines[index].trim().match(BULLET);
    if (!match) {
      break;
    }
    bullets.push(match[1].trim());
  }
  return bullets;
}

/**
 * Parse every declaration line in the message, for any addressee. Options
 * are adopted only when the immediately following consecutive bullet run
 * has 2-4 items; a blank line detaches the list and 5+ items or a lone
 * bullet are treated as context, not answers.
 */
export function parseDeclaredAsks(content: string): DeclaredAsk[] {
  const lines = content.split("\n");
  const asks: DeclaredAsk[] = [];
  for (let index = 0; index < lines.length; index++) {
    const declaration = parseDeclarationLine(lines[index]);
    if (!declaration) {
      continue;
    }
    const bullets = collectBullets(lines, index + 1);
    const options =
      bullets.length >= MIN_OPTIONS && bullets.length <= MAX_OPTIONS
        ? bullets
        : [];
    asks.push({ ...declaration, options });
  }
  return asks;
}

/**
 * The declarations addressed to the viewer: the declared name equals the
 * viewer display name or its first word, case-insensitively ("Needs Lee"
 * matches viewer "Lee Ntshudisane").
 */
export function viewerAsks(
  asks: DeclaredAsk[],
  viewerName: string | undefined,
): DeclaredAsk[] {
  const full = viewerName?.trim().toLowerCase() ?? "";
  if (!full) {
    return [];
  }
  const first = full.split(/\s+/)[0];
  return asks.filter((entry) => {
    const name = entry.name.trim().toLowerCase();
    return name === full || name === first;
  });
}

/**
 * Remove every declaration line and its attached bullet run so the derived
 * tier cannot re-surface asks addressed to other people. A message with
 * only non-viewer declarations therefore classifies headsUp unless the
 * remaining prose contains its own ask.
 */
export function stripDeclaredAskLines(content: string): string {
  const lines = content.split("\n");
  const kept: string[] = [];
  let index = 0;
  while (index < lines.length) {
    if (parseDeclarationLine(lines[index])) {
      index += 1 + collectBullets(lines, index + 1).length;
      continue;
    }
    kept.push(lines[index]);
    index++;
  }
  return kept.join("\n");
}
