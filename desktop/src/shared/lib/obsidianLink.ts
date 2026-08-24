const OBSIDIAN_OPEN_PARAMS = new Set(["file", "path", "vault"]);
const MAX_OBSIDIAN_LINK_LENGTH = 8_192;

function hasControlCharacters(value: string): boolean {
  return Array.from(value).some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || codePoint === 0x7f;
  });
}

/**
 * Accept only Obsidian's read-oriented `open` action. Other Obsidian URI
 * actions can create notes, run searches, or invoke plugin-defined behavior,
 * so message links must never pass arbitrary `obsidian://` URLs to the OS.
 */
export function isObsidianOpenLink(value: string): boolean {
  if (!value || value.length > MAX_OBSIDIAN_LINK_LENGTH) return false;

  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return false;
  }

  if (
    url.protocol !== "obsidian:" ||
    url.hostname !== "open" ||
    (url.pathname !== "" && url.pathname !== "/") ||
    url.username ||
    url.password ||
    url.port ||
    url.hash
  ) {
    return false;
  }

  for (const key of url.searchParams.keys()) {
    if (!OBSIDIAN_OPEN_PARAMS.has(key)) return false;
  }
  for (const key of OBSIDIAN_OPEN_PARAMS) {
    if (url.searchParams.getAll(key).length > 1) return false;
  }

  const vault = url.searchParams.get("vault")?.trim() ?? "";
  const file = url.searchParams.get("file")?.trim() ?? "";
  const path = url.searchParams.get("path")?.trim() ?? "";
  if ([vault, file, path].some(hasControlCharacters)) return false;

  // `path` is the absolute-path form. `vault`/`file` address a vault-relative
  // target and may be used individually or together.
  return path ? !vault && !file : Boolean(vault || file);
}
