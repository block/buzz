export function shortenProjectPath(path: string, maxSegments = 3) {
  const trimmed = path.trim();
  if (!trimmed) return "";
  const normalized = trimmed.replaceAll("\\", "/").replace(/\/+$/, "");
  const segments = normalized.split("/").filter(Boolean);
  if (segments.length <= maxSegments) return normalized;
  return `…/${segments.slice(-maxSegments).join("/")}`;
}
