import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_STICKER_CATALOG,
  KIND_STICKER_CATALOG_COMMAND,
  KIND_STICKER_PACK,
  KIND_USER_STICKER_PACKS,
} from "@/shared/constants/kinds";

export const SONAR_PACK_FORMAT = "sonar-sticker-pack-v1";
export const MAX_STICKERS_PER_PACK = 200;
export const MAX_STICKER_CATALOG_PACKS = 500;

const PUBKEY_RE = /^[0-9a-f]{64}$/;
const HASH_RE = /^[0-9a-f]{64}$/;
const IDENTIFIER_RE = /^[A-Za-z0-9._-]{1,80}$/;
const SHORTCODE_RE = /^[A-Za-z0-9_]{1,64}$/;
const ALLOWED_MIMES = new Set([
  "image/webp",
  "image/png",
  "image/apng",
  "image/gif",
]);

export type StickerAsset = {
  shortcode: string;
  url: string;
  sha256: string;
  mime: string;
  width?: number;
  height?: number;
  alt?: string;
  emoji?: string;
};

export type StickerCover = { url: string; sha256: string; dim?: string };

export type StickerPack = {
  coordinate: string;
  author: string;
  identifier: string;
  title: string;
  description?: string;
  license?: string;
  cover?: StickerCover;
  stickers: StickerAsset[];
  eventId: string;
};

export type StickerReference = {
  coordinate: string;
  author: string;
  identifier: string;
  shortcode: string;
  sha256: string;
};

export type CatalogEntry = {
  coordinate: string;
  approvedEventId: string;
};

export type ImportedStickerDraft = {
  identifier: string;
  title: string;
  author?: string;
  cover?: StickerAsset;
  stickers: StickerAsset[];
  skippedStickerIds: number[];
};

function firstTag(event: RelayEvent, name: string): string[] | undefined {
  return event.tags.find((tag) => tag[0] === name);
}

function parseDim(value: string | undefined): [number?, number?] {
  if (!value) return [];
  const match = /^(\d+)x(\d+)$/.exec(value);
  if (!match) return [];
  const width = Number(match[1]);
  const height = Number(match[2]);
  if (width < 1 || height < 1 || width > 4096 || height > 4096) return [];
  return [width, height];
}

export function parsePackCoordinate(
  coordinate: string,
): { author: string; identifier: string } | null {
  const [kind, rawAuthor, ...identifierParts] = coordinate.split(":");
  const author = rawAuthor?.toLowerCase();
  const identifier = identifierParts.join(":");
  if (
    kind !== String(KIND_STICKER_PACK) ||
    rawAuthor !== author ||
    !PUBKEY_RE.test(author ?? "") ||
    !IDENTIFIER_RE.test(identifier)
  ) {
    return null;
  }
  return { author: author as string, identifier };
}

function isHttpsHashUrl(url: string, sha256: string): boolean {
  try {
    const parsed = new URL(url);
    return (
      parsed.protocol === "https:" &&
      parsed.username === "" &&
      parsed.password === "" &&
      parsed.port === "" &&
      parsed.pathname.includes(sha256)
    );
  } catch {
    return false;
  }
}

export function parseStickerPack(event: RelayEvent): StickerPack | null {
  if (event.kind !== KIND_STICKER_PACK || event.content !== "") return null;
  const author = event.pubkey.toLowerCase();
  const identifier = firstTag(event, "d")?.[1] ?? "";
  const title = firstTag(event, "title")?.[1]?.trim() ?? "";
  const format = firstTag(event, "pack_format")?.[1];
  const exactSingleton = (name: string, value?: string) => {
    const matching = event.tags.filter((tag) => tag[0] === name);
    return (
      matching.length === 1 &&
      matching[0].length === 2 &&
      (value === undefined || matching[0][1] === value)
    );
  };
  const optionalExactSingleton = (name: string, value: string) => {
    const matching = event.tags.filter((tag) => tag[0] === name);
    return (
      matching.length === 0 ||
      (matching.length === 1 &&
        matching[0].length === 2 &&
        matching[0][1] === value)
    );
  };
  if (
    event.pubkey !== author ||
    !PUBKEY_RE.test(author) ||
    !IDENTIFIER_RE.test(identifier) ||
    format !== SONAR_PACK_FORMAT ||
    title.length === 0 ||
    [...title].length > 80 ||
    !exactSingleton("d", identifier) ||
    !exactSingleton("title", title) ||
    !exactSingleton("pack_format", SONAR_PACK_FORMAT) ||
    !optionalExactSingleton("t", SONAR_PACK_FORMAT)
  ) {
    return null;
  }

  const seenShortcodes = new Set<string>();
  const seenHashes = new Set<string>();
  const stickers: StickerAsset[] = [];
  for (const tag of event.tags) {
    if (tag[0] !== "sticker") continue;
    if (tag.length < 6 || tag.length > 8) return null;
    const [, shortcode, url, rawHash, rawMime, dim, alt, emoji] = tag;
    const sha256 = rawHash?.toLowerCase() ?? "";
    const mime = rawMime?.toLowerCase() ?? "";
    const [width, height] = parseDim(dim);
    if (
      !SHORTCODE_RE.test(shortcode ?? "") ||
      rawHash !== sha256 ||
      !HASH_RE.test(sha256) ||
      !ALLOWED_MIMES.has(mime) ||
      !isHttpsHashUrl(url ?? "", sha256) ||
      (dim && (width === undefined || height === undefined)) ||
      (alt && [...alt].length > 160) ||
      (emoji && [...emoji].length > 8) ||
      seenShortcodes.has(shortcode) ||
      seenHashes.has(sha256)
    ) {
      return null;
    }
    seenShortcodes.add(shortcode);
    seenHashes.add(sha256);
    stickers.push({
      shortcode,
      url,
      sha256,
      mime,
      ...(width !== undefined && height !== undefined ? { width, height } : {}),
      ...(alt ? { alt } : {}),
      ...(emoji ? { emoji } : {}),
    });
  }
  if (stickers.length < 1 || stickers.length > MAX_STICKERS_PER_PACK) {
    return null;
  }
  const compatibilityTags = event.tags.filter((tag) => tag[0] === "emoji");
  if (
    compatibilityTags.some((tag, index) => {
      if (tag.length !== 3) return true;
      const sticker = stickers.find((item) => item.shortcode === tag[1]);
      return (
        !sticker ||
        sticker.url !== tag[2] ||
        compatibilityTags.findIndex(
          (other) => other[1] === tag[1] && other[2] === tag[2],
        ) !== index
      );
    })
  ) {
    return null;
  }

  const description = firstTag(event, "description")?.[1];
  if (
    (description && [...description].length > 500) ||
    event.tags
      .filter((tag) => tag[0] === "description")
      .some((tag) => tag.length !== 2) ||
    event.tags.filter((tag) => tag[0] === "description").length > 1
  )
    return null;
  const license = firstTag(event, "license")?.[1];
  if (
    (license && [...license].length > 160) ||
    event.tags
      .filter((tag) => tag[0] === "license")
      .some((tag) => tag.length !== 2) ||
    event.tags.filter((tag) => tag[0] === "license").length > 1
  )
    return null;
  const image = firstTag(event, "image");
  if (
    event.tags.filter((tag) => tag[0] === "image").length > 1 ||
    (image && image.length !== 3 && image.length !== 4)
  )
    return null;
  let cover: StickerPack["cover"];
  if (image) {
    const hash = image[2]?.toLowerCase() ?? "";
    const [width, height] = parseDim(image[3]);
    if (
      image[2] !== hash ||
      !HASH_RE.test(hash) ||
      !isHttpsHashUrl(image[1] ?? "", hash) ||
      (image[3] && (width === undefined || height === undefined))
    ) {
      return null;
    }
    cover = {
      url: image[1],
      sha256: hash,
      ...(image[3] ? { dim: image[3] } : {}),
    };
  }
  return {
    coordinate: `${KIND_STICKER_PACK}:${author}:${identifier}`,
    author,
    identifier,
    title,
    ...(description ? { description } : {}),
    ...(license ? { license } : {}),
    ...(cover ? { cover } : {}),
    stickers,
    eventId: event.id,
  };
}

export function parseStickerReference(
  tags: ReadonlyArray<ReadonlyArray<string>> | undefined,
): StickerReference | null {
  const stickerTags = (tags ?? []).filter((tag) => tag[0] === "sticker");
  if (stickerTags.length !== 1 || stickerTags[0].length !== 4) return null;
  const [, coordinate, shortcode, rawHash] = stickerTags[0];
  const address = coordinate ? parsePackCoordinate(coordinate) : null;
  const sha256 = rawHash?.toLowerCase() ?? "";
  if (
    !address ||
    rawHash !== sha256 ||
    !SHORTCODE_RE.test(shortcode ?? "") ||
    !HASH_RE.test(sha256)
  ) {
    return null;
  }
  return { coordinate, ...address, shortcode, sha256 };
}

export function stickerReferenceTag(
  pack: StickerPack,
  sticker: StickerAsset,
): string[] {
  return ["sticker", pack.coordinate, sticker.shortcode, sticker.sha256];
}

export function stickerCacheUrl(reference: StickerReference): string {
  return `/media/sticker/${encodeURIComponent(reference.author)}/${encodeURIComponent(reference.identifier)}/${encodeURIComponent(reference.shortcode)}/${reference.sha256}`;
}

export function stickerAssetCacheUrl(
  pack: StickerPack,
  sticker: StickerAsset,
): string {
  return stickerCacheUrl({
    coordinate: pack.coordinate,
    author: pack.author,
    identifier: pack.identifier,
    shortcode: sticker.shortcode,
    sha256: sticker.sha256,
  });
}

export function catalogEntriesFromEvent(
  event: RelayEvent | null,
): CatalogEntry[] {
  if (!event || event.kind !== KIND_STICKER_CATALOG || event.content !== "")
    return [];
  if (
    event.tags.filter((tag) => tag.length === 1 && tag[0] === "-").length !== 1
  )
    return [];
  const seen = new Set<string>();
  const entries: CatalogEntry[] = [];
  for (const tag of event.tags) {
    if (tag.length === 1 && tag[0] === "-") continue;
    if (tag[0] !== "a") return [];
    if (tag.length !== 3) return [];
    const [, coordinate, approvedEventId] = tag;
    if (
      !coordinate ||
      !parsePackCoordinate(coordinate) ||
      !approvedEventId ||
      approvedEventId !== approvedEventId.toLowerCase() ||
      !HASH_RE.test(approvedEventId.toLowerCase()) ||
      seen.has(coordinate)
    ) {
      return [];
    }
    seen.add(coordinate);
    entries.push({
      coordinate,
      approvedEventId: approvedEventId.toLowerCase(),
    });
    if (entries.length > MAX_STICKER_CATALOG_PACKS) return [];
  }
  return entries;
}

export async function fetchStickerCatalog(): Promise<StickerPack[]> {
  const snapshots = await relayClient.fetchEvents({
    kinds: [KIND_STICKER_CATALOG],
    limit: 1,
  });
  const entries = catalogEntriesFromEvent(snapshots.at(-1) ?? null);
  if (entries.length === 0) return [];
  const events = await relayClient.fetchEvents({
    kinds: [KIND_STICKER_PACK],
    ids: entries.map((entry) => entry.approvedEventId),
    limit: entries.length,
  });
  const byId = new Map(events.map((event) => [event.id.toLowerCase(), event]));
  return entries.flatMap((entry) => {
    const event = byId.get(entry.approvedEventId);
    const pack = event ? parseStickerPack(event) : null;
    return pack?.coordinate === entry.coordinate ? [pack] : [];
  });
}

export async function fetchAllStickerPacks(): Promise<StickerPack[]> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_STICKER_PACK],
    limit: MAX_STICKER_CATALOG_PACKS,
  });
  return events
    .map(parseStickerPack)
    .filter((pack): pack is StickerPack => pack !== null)
    .sort((a, b) => a.title.localeCompare(b.title));
}

export async function fetchOwnStickerPacks(): Promise<StickerPack[]> {
  const { pubkey } = await getIdentity();
  const events = await relayClient.fetchEvents({
    kinds: [KIND_STICKER_PACK],
    authors: [pubkey],
    limit: 200,
  });
  return events
    .map(parseStickerPack)
    .filter((pack): pack is StickerPack => pack !== null)
    .sort((a, b) => a.title.localeCompare(b.title));
}

export async function fetchInstalledPackCoordinates(): Promise<string[]> {
  const { pubkey } = await getIdentity();
  const events = await relayClient.fetchEvents({
    kinds: [KIND_USER_STICKER_PACKS],
    authors: [pubkey],
    limit: 1,
  });
  const event = events.at(-1);
  if (event?.content !== "" || event.pubkey !== pubkey) return [];
  const seen = new Set<string>();
  const coordinates: string[] = [];
  for (const tag of event.tags) {
    if (tag.length !== 2 || tag[0] !== "a") return [];
    const coordinate = tag[1];
    if (
      !coordinate ||
      !parsePackCoordinate(coordinate) ||
      seen.has(coordinate)
    ) {
      return [];
    }
    seen.add(coordinate);
    coordinates.push(coordinate);
  }
  return coordinates;
}

async function publishInstalledPackCoordinates(
  coordinates: string[],
): Promise<void> {
  const event = await signRelayEvent({
    kind: KIND_USER_STICKER_PACKS,
    content: "",
    tags: coordinates.map((coordinate) => ["a", coordinate]),
  });
  await relayClient.publishEvent(
    event,
    "Timed out updating stickers.",
    "Failed to update stickers.",
  );
  const { pubkey } = await getIdentity();
  const current = await relayClient.fetchEvents({
    kinds: [KIND_USER_STICKER_PACKS],
    authors: [pubkey],
    limit: 1,
  });
  if (current.at(-1)?.id !== event.id) {
    throw new Error(
      "Your installed sticker list changed concurrently. Refresh and try again.",
    );
  }
}

export async function setStickerPackInstalled(
  coordinate: string,
  installed: boolean,
): Promise<void> {
  if (!parsePackCoordinate(coordinate))
    throw new Error("Invalid sticker pack coordinate.");
  const current = await fetchInstalledPackCoordinates();
  const next = new Set(current);
  if (installed) next.add(coordinate);
  else next.delete(coordinate);
  await publishInstalledPackCoordinates([...next]);
}

export async function publishStickerPack(input: {
  identifier: string;
  title: string;
  description?: string;
  license?: string;
  cover?: StickerCover;
  stickers: StickerAsset[];
}): Promise<void> {
  if (!IDENTIFIER_RE.test(input.identifier))
    throw new Error(
      "Pack ID must use letters, numbers, dot, dash, or underscore.",
    );
  const title = input.title.trim();
  if (!title || [...title].length > 80)
    throw new Error("Pack title must be 1–80 characters.");
  if (
    input.stickers.length < 1 ||
    input.stickers.length > MAX_STICKERS_PER_PACK
  )
    throw new Error("A pack must contain 1–200 stickers.");
  if (input.description && [...input.description].length > 500)
    throw new Error("Description must be at most 500 characters.");
  if (input.license && [...input.license].length > 160)
    throw new Error("License must be at most 160 characters.");
  const seenShortcodes = new Set<string>();
  const seenHashes = new Set<string>();
  const tags: string[][] = [
    ["d", input.identifier],
    ["title", title],
    ["pack_format", SONAR_PACK_FORMAT],
    ["t", SONAR_PACK_FORMAT],
  ];
  if (input.description?.trim())
    tags.push(["description", input.description.trim()]);
  if (input.license?.trim()) tags.push(["license", input.license.trim()]);
  if (input.cover) {
    const coverHash = input.cover.sha256.toLowerCase();
    const [width, height] = parseDim(input.cover.dim);
    if (
      input.cover.sha256 !== coverHash ||
      !HASH_RE.test(coverHash) ||
      !isHttpsHashUrl(input.cover.url, coverHash) ||
      (input.cover.dim && (width === undefined || height === undefined))
    ) {
      throw new Error(
        "The sticker pack cover is not a valid Sonar WebP asset.",
      );
    }
    tags.push([
      "image",
      input.cover.url,
      coverHash,
      ...(input.cover.dim ? [input.cover.dim] : []),
    ]);
  }
  for (const sticker of input.stickers) {
    if (
      !SHORTCODE_RE.test(sticker.shortcode) ||
      sticker.sha256 !== sticker.sha256.toLowerCase() ||
      !HASH_RE.test(sticker.sha256) ||
      sticker.mime !== sticker.mime.toLowerCase() ||
      !ALLOWED_MIMES.has(sticker.mime) ||
      !isHttpsHashUrl(sticker.url, sticker.sha256) ||
      (sticker.width === undefined) !== (sticker.height === undefined) ||
      (sticker.width !== undefined &&
        (sticker.width < 1 || sticker.width > 4096)) ||
      (sticker.height !== undefined &&
        (sticker.height < 1 || sticker.height > 4096)) ||
      (sticker.alt !== undefined && [...sticker.alt].length > 160) ||
      (sticker.emoji !== undefined && [...sticker.emoji].length > 8) ||
      seenShortcodes.has(sticker.shortcode) ||
      seenHashes.has(sticker.sha256)
    ) {
      throw new Error(
        `Sticker :${sticker.shortcode}: is not a valid Sonar HTTPS asset.`,
      );
    }
    seenShortcodes.add(sticker.shortcode);
    seenHashes.add(sticker.sha256);
    tags.push([
      "sticker",
      sticker.shortcode,
      sticker.url,
      sticker.sha256,
      sticker.mime,
      sticker.width && sticker.height
        ? `${sticker.width}x${sticker.height}`
        : "",
      sticker.alt ?? "",
      ...(sticker.emoji ? [sticker.emoji] : []),
    ]);
    tags.push(["emoji", sticker.shortcode, sticker.url]);
  }
  const event = await signRelayEvent({
    kind: KIND_STICKER_PACK,
    content: "",
    tags,
  });
  await relayClient.publishEvent(
    event,
    "Timed out publishing sticker pack.",
    "Failed to publish sticker pack.",
  );
  const current = await relayClient.fetchEvents({
    kinds: [KIND_STICKER_PACK],
    authors: [event.pubkey],
    "#d": [input.identifier],
    limit: 1,
  });
  if (current.at(-1)?.id !== event.id) {
    throw new Error(
      "This sticker pack changed concurrently. Refresh and try again.",
    );
  }
}

export async function setStickerCatalogApproval(
  coordinate: string,
  approvedEventId: string,
  approved: boolean,
): Promise<void> {
  if (!parsePackCoordinate(coordinate) || !HASH_RE.test(approvedEventId))
    throw new Error("Invalid sticker pack.");
  const event = await signRelayEvent({
    kind: KIND_STICKER_CATALOG_COMMAND,
    content: "",
    tags: [
      ["action", approved ? "approve" : "remove"],
      approved ? ["a", coordinate, approvedEventId] : ["a", coordinate],
    ],
  });
  await relayClient.publishEvent(
    event,
    "Timed out updating sticker catalog.",
    "Failed to update sticker catalog.",
  );
}
