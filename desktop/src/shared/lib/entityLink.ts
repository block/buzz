/**
 * `buzz://` deep links for Buzz-hosted git entities, mirroring
 * `features/messages/lib/messageLink.ts` for `buzz://message`.
 *
 * Formats:
 *   buzz://repo?owner=<owner-pubkey>&d=<repo-dtag>
 *   buzz://pr?id=<event-id>&owner=<owner-pubkey>&d=<repo-dtag>
 *   buzz://issue?id=<event-id>&owner=<owner-pubkey>&d=<repo-dtag>
 *
 * `owner` + `d` identify the NIP-34 repository coordinate
 * (`30617:<owner>:<d>`); `id` is the kind 1618 / 1621 event id. The CLI
 * builder in `crates/buzz-cli/src/links.rs` emits the same format — the two
 * must stay compatible (see the golden-format tests on both sides).
 */

const ENTITY_LINK_SCHEME = "buzz:";

export type ParsedEntityLink =
  | { type: "pr"; id: string; owner: string; dtag: string }
  | { type: "issue"; id: string; owner: string; dtag: string }
  | { type: "repo"; owner: string; dtag: string };

export type EntityLinkParseResult =
  | { ok: true; value: ParsedEntityLink }
  | { ok: false; reason: string };

const HEX64_RE = /^[a-fA-F0-9]{64}$/;
const DTAG_RE = /^[a-zA-Z0-9._-]{1,64}$/;

function isValidDtag(dtag: string): boolean {
  return DTAG_RE.test(dtag) && !dtag.startsWith(".") && !dtag.includes("..");
}

function checkCoordinate(owner: string, dtag: string): void {
  if (!HEX64_RE.test(owner)) {
    throw new Error("entityLink: owner must be a 64-char hex pubkey");
  }
  if (!isValidDtag(dtag)) {
    throw new Error("entityLink: invalid repository d-tag");
  }
}

function checkEventId(id: string): void {
  if (!HEX64_RE.test(id)) {
    throw new Error("entityLink: id must be a 64-char hex event id");
  }
}

/** Build a `buzz://repo` link for a repository announcement (kind 30617). */
export function buildRepoLink(input: { owner: string; dtag: string }): string {
  checkCoordinate(input.owner, input.dtag);
  return `buzz://repo?owner=${input.owner.toLowerCase()}&d=${input.dtag}`;
}

/** Build a `buzz://pr` link for a pull request event (kind 1618). */
export function buildPullRequestLink(input: {
  id: string;
  owner: string;
  dtag: string;
}): string {
  checkEventId(input.id);
  checkCoordinate(input.owner, input.dtag);
  return `buzz://pr?id=${input.id.toLowerCase()}&owner=${input.owner.toLowerCase()}&d=${input.dtag}`;
}

/** Build a `buzz://issue` link for an issue event (kind 1621). */
export function buildIssueLink(input: {
  id: string;
  owner: string;
  dtag: string;
}): string {
  checkEventId(input.id);
  checkCoordinate(input.owner, input.dtag);
  return `buzz://issue?id=${input.id.toLowerCase()}&owner=${input.owner.toLowerCase()}&d=${input.dtag}`;
}

/**
 * Cheap pre-check used by the markdown renderer and preview extraction
 * before parsing. `buzz://message` is intentionally excluded — it has its
 * own pill rendering path.
 */
export function isEntityLink(href: string | undefined | null): boolean {
  if (!href) return false;
  return (
    href.startsWith("buzz://pr?") ||
    href.startsWith("buzz://issue?") ||
    href.startsWith("buzz://repo?")
  );
}

/**
 * Parse a `buzz://pr|issue|repo?…` URL. Returns a discriminated result so
 * callers can fall back to plain-link rendering without throwing. All
 * identifiers are validated; hex values are lowercase-normalized.
 */
export function parseEntityLink(url: string): EntityLinkParseResult {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }

  if (parsed.protocol !== ENTITY_LINK_SCHEME) {
    return { ok: false, reason: "wrong-scheme" };
  }

  const host = parsed.hostname;
  if (host !== "pr" && host !== "issue" && host !== "repo") {
    return { ok: false, reason: "wrong-host" };
  }

  const owner = parsed.searchParams.get("owner");
  const dtag = parsed.searchParams.get("d");
  if (!owner || !HEX64_RE.test(owner)) {
    return { ok: false, reason: "invalid-owner" };
  }
  if (!dtag || !isValidDtag(dtag)) {
    return { ok: false, reason: "invalid-dtag" };
  }

  if (host === "repo") {
    return {
      ok: true,
      value: { type: "repo", owner: owner.toLowerCase(), dtag },
    };
  }

  const id = parsed.searchParams.get("id");
  if (!id || !HEX64_RE.test(id)) {
    return { ok: false, reason: "invalid-id" };
  }

  return {
    ok: true,
    value: {
      type: host,
      id: id.toLowerCase(),
      owner: owner.toLowerCase(),
      dtag,
    },
  };
}

/**
 * Route id accepted by `/projects/$projectId` (`<owner-pubkey>:<dtag>`, see
 * `parseProjectRouteId` in `features/projects/hooks.ts`).
 */
export function entityLinkProjectRouteId(link: ParsedEntityLink): string {
  return `${link.owner}:${link.dtag}`;
}
