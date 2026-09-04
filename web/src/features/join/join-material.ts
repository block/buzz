/**
 * Join-by-address pairing material — `join.json` at the relay origin.
 *
 * The always-on relay serves the community's pairing material ITSELF, so a
 * stranger holding nothing but the address can join with no second machine:
 * no desktop to pair against, no invite code handed over by a member.
 *
 * The file is operator-gated by construction: it is served from the relay's
 * own origin, and an operator who has not published it simply has none —
 * the client fails closed with a plain refusal, never a guess. Fail-closed
 * on unknown hosts is untouched: the client connects only to the address it
 * was given, and the relay still refuses what its policy refuses.
 *
 * Shape (v1):
 * {
 *   "v": 1,
 *   "community": { "host": "skaists.buzz", "name": "skaists" },
 *   "invite_url": "/invite/v2…",          // the standing invite
 *   "default_channel": { "id": "<uuid>", "name": "welcome-everyone" },
 *   "note": "optional human line shown on the join page"
 * }
 */

export type JoinMaterial = {
  v: 1;
  community: { host: string; name?: string };
  invite_url: string;
  default_channel: { id: string; name?: string };
  note?: string;
};

const FETCH_TIMEOUT_MS = 8000;

/** Fetch the community's pairing material; `null` = none published (fail closed). */
export async function fetchJoinMaterial(
  origin: string,
): Promise<JoinMaterial | null> {
  const url = `${origin.replace(/\/+$/, "")}/join.json`;
  try {
    const response = await fetch(url, {
      headers: { Accept: "application/json" },
      signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
    });
    if (response.status === 404) return null;
    if (!response.ok) return null;
    const json = (await response.json()) as JoinMaterial;
    if (
      json?.v !== 1 ||
      typeof json.invite_url !== "string" ||
      !json.invite_url ||
      typeof json.default_channel?.id !== "string" ||
      !json.default_channel.id
    ) {
      return null;
    }
    return json;
  } catch {
    return null;
  }
}

/** Extract the invite code from the material's invite_url (path or absolute). */
export function inviteCodeFromMaterial(material: JoinMaterial): string | null {
  let path = material.invite_url;
  const marker = path.indexOf("/invite/");
  if (marker === -1) return null;
  path = path.slice(marker + "/invite/".length);
  const code = path.split(/[#?]/)[0];
  return code ? decodeURIComponent(code) : null;
}
