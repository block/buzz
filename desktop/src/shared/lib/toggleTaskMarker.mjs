/**
 * Flip the Nth GFM task-list marker (`- [ ]` / `- [x]`) in Markdown source.
 *
 * Runtime lives in `.mjs` so the `node:test` runner can import it directly;
 * `toggleTaskMarker.d.mts` gives TypeScript callers a typed view.
 *
 * The rendered checkbox carries no source position — remark-gfm *generates*
 * the `<input>` during the mdast→hast conversion, so there is no offset to
 * map a click back to the marker that produced it. `rehypeTaskIndex` stamps
 * each checkbox with its ordinal instead, and this function resolves that
 * ordinal against the source. The two must therefore count the same markers:
 * that is why fenced code blocks are skipped here (a `- [ ]` inside a fence
 * renders as code, never as a checkbox) and why the marker must be followed
 * by whitespace or end-of-line, matching the GFM rule remark applies.
 */

// A list bullet (`-`, `*`, `+`, or `1.` / `1)`) followed by `[ ]` or `[x]`.
// Not global: applied per line, so `exec` has no lastIndex to carry.
const TASK_MARKER = /^([ \t]*(?:[-*+]|\d{1,9}[.)])[ \t]+)\[[ xX]\](?=[ \t]|$)/;

// Opening/closing fence of a code block: three or more backticks or tildes.
const FENCE = /^[ \t]*(`{3,}|~{3,})/;

/**
 * @param {string} source Raw Markdown of the message.
 * @param {number} taskIndex Zero-based ordinal from `rehypeTaskIndex`.
 * @param {boolean} checked Desired state of that task.
 * @returns {string | null} The updated source, or `null` when the ordinal
 *   resolves to nothing — which means the source no longer matches what was
 *   rendered (a concurrent edit landed). Callers must not write a guess.
 */
export function toggleTaskMarker(source, taskIndex, checked) {
  if (typeof source !== "string") return null;
  if (!Number.isInteger(taskIndex) || taskIndex < 0) return null;

  const lines = source.split("\n");
  /** Fence character currently open (`` ` `` or `~`), or null outside a fence. */
  let openFence = null;
  let seen = 0;

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];

    const fence = FENCE.exec(line);
    if (fence) {
      const char = fence[1][0];
      if (openFence === null) openFence = char;
      else if (openFence === char) openFence = null;
      continue;
    }
    if (openFence !== null) continue;

    const marker = TASK_MARKER.exec(line);
    if (!marker) continue;
    if (seen !== taskIndex) {
      seen += 1;
      continue;
    }

    const bullet = marker[1];
    const rest = line.slice(marker[0].length);
    lines[i] = `${bullet}[${checked ? "x" : " "}]${rest}`;
    return lines.join("\n");
  }

  return null;
}

/**
 * Number of task markers the renderer will turn into checkboxes. Exposed so
 * callers can sanity-check an ordinal before writing.
 *
 * @param {string} source
 * @returns {number}
 */
export function countTaskMarkers(source) {
  if (typeof source !== "string") return 0;
  let openFence = null;
  let count = 0;
  for (const line of source.split("\n")) {
    const fence = FENCE.exec(line);
    if (fence) {
      const char = fence[1][0];
      if (openFence === null) openFence = char;
      else if (openFence === char) openFence = null;
      continue;
    }
    if (openFence !== null) continue;
    if (TASK_MARKER.test(line)) count += 1;
  }
  return count;
}
