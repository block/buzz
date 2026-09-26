/** Cursor-bridge progress lines that should not each take a chat row. */

const PROGRESS_PREFIX =
  /^(?:▸ Working\b|▸ Cursor\b|✅ (?:Cursor|Done)\b|⚠ Cursor\b|⏳ |⚙ |✓ |✗ |📣 |💭 )/;

export function isAgentProgressBody(body: string | null | undefined): boolean {
  const text = (body ?? "").trim();
  if (!text) {
    return false;
  }
  return PROGRESS_PREFIX.test(text);
}

export function agentProgressLatestLabel(
  bodies: readonly (string | null | undefined)[],
): string {
  for (let i = bodies.length - 1; i >= 0; i -= 1) {
    const text = (bodies[i] ?? "").trim();
    if (text) {
      return text.length > 120 ? `${text.slice(0, 117)}…` : text;
    }
  }
  return "Working";
}

export function agentProgressIsActive(
  latestBody: string | null | undefined,
  latestCreatedAt: number,
  nowSeconds: number,
): boolean {
  const text = (latestBody ?? "").trim();
  if (/^✅ (?:Cursor|Done)\b/.test(text) || text.startsWith("✗ ")) {
    return false;
  }
  return nowSeconds - latestCreatedAt < 120;
}
