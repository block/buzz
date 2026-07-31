/**
 * Mock Sonar sticker-pack fixtures served by the e2e bridge's mock relay.
 *
 * Lives beside `e2eBridge.ts` rather than inside it: the bridge is already
 * enormous, and these fixtures are pure data plus a couple of tiny builders,
 * so keeping them separate lets Playwright specs import the same seed the
 * bridge serves (e.g. to fulfil the sticker asset requests with real image
 * bytes) without pulling in the whole bridge module.
 *
 * Every value here is shaped to survive `parseStickerPack` /
 * `catalogEntriesFromEvent` / `fetchInstalledPackCoordinates` in
 * `@/shared/api/stickers`. The strictest constraint is `isHttpsHashUrl`: each
 * asset URL must be `https:`, carry no port and no credentials, and contain
 * the lowercase sha256 in its pathname — so the fixture URLs are built by
 * {@link stickerAssetUrl} rather than written by hand.
 *
 * The UI never fetches these `https:` URLs. Both the picker and the timeline
 * render the relay cache route (`stickerCacheUrl`), which `rewriteRelayUrl`
 * turns into `http://127.0.0.1:<mock proxy port>/media/sticker/…`. Specs route
 * that pattern to serve the actual pixels.
 */
import type { RelayEvent } from "@/shared/api/types";

/** `KIND_STICKER_PACK` — duplicated to keep this module dependency-free. */
const KIND_STICKER_PACK = 30031;

/** Fixed host for pack asset URLs. Never fetched (see module docs). */
const STICKER_ASSET_HOST = "https://stickers.buzz.example";

export const SONAR_PACK_FORMAT = "sonar-sticker-pack-v1";

/** The mock bridge's default identity (`DEFAULT_MOCK_IDENTITY.pubkey`). */
const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);
const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const BOB_PUBKEY =
  "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260";

export type MockStickerSeed = {
  shortcode: string;
  alt: string;
  emoji: string;
};

export type MockStickerPackSeed = {
  /** Deterministic 64-hex event id — the catalog references packs by id. */
  eventId: string;
  author: string;
  identifier: string;
  title: string;
  description: string;
  license?: string;
  createdAt: number;
  stickers: MockStickerSeed[];
};

/**
 * Deterministic 64-hex asset digest.
 *
 * `slot` separates packs and `index` separates stickers inside a pack, so no
 * two fixture assets share a hash (`parseStickerPack` rejects duplicates).
 */
function assetHash(slot: number, index: number): string {
  const head = `${slot.toString(16).padStart(2, "0")}${index
    .toString(16)
    .padStart(2, "0")}`;
  return `${head}${"5a".repeat(30)}`;
}

/**
 * Build an asset URL that satisfies `isHttpsHashUrl` — https, no port, no
 * credentials, digest present in the pathname.
 */
export function stickerAssetUrl(
  author: string,
  identifier: string,
  shortcode: string,
  sha256: string,
): string {
  return `${STICKER_ASSET_HOST}/media/sticker/${author}/${identifier}/${shortcode}/${sha256}`;
}

/** Base timestamp so fixture ordering is stable across runs. */
const SEED_CREATED_AT = 1_750_000_000;

export const MOCK_STICKER_PACK_SEEDS: MockStickerPackSeed[] = [
  {
    eventId: `b0${"b0".repeat(31)}`,
    author: MOCK_IDENTITY_PUBKEY,
    identifier: "buzz-classics",
    title: "Buzz Classics",
    description:
      "The original Buzz reaction set — hand-drawn, always on brand.",
    license: "CC BY-SA 4.0",
    createdAt: SEED_CREATED_AT,
    stickers: [
      { shortcode: "bee_hello", alt: "Bee waving hello", emoji: "👋" },
      { shortcode: "bee_party", alt: "Bee with a party hat", emoji: "🎉" },
      { shortcode: "ship_it", alt: "Rocket labelled ship it", emoji: "🚀" },
      { shortcode: "lgtm", alt: "Thumbs up, looks good to me", emoji: "👍" },
      { shortcode: "thinking", alt: "Bee deep in thought", emoji: "🤔" },
      { shortcode: "on_fire", alt: "Bee on fire", emoji: "🔥" },
      { shortcode: "coffee", alt: "Bee holding a coffee", emoji: "☕" },
      { shortcode: "heart_bee", alt: "Bee hugging a heart", emoji: "💛" },
      { shortcode: "eyes", alt: "Bee watching closely", emoji: "👀" },
      { shortcode: "sleepy", alt: "Sleepy bee", emoji: "😴" },
    ],
  },
  {
    eventId: `c1${"c1".repeat(31)}`,
    author: ALICE_PUBKEY,
    identifier: "release-day",
    title: "Release Day",
    description: "Deploys, rollbacks, and everything between green and red.",
    license: "MIT",
    createdAt: SEED_CREATED_AT + 60,
    stickers: [
      { shortcode: "deploying", alt: "Deploy in progress", emoji: "📦" },
      { shortcode: "green_build", alt: "All checks passed", emoji: "✅" },
      { shortcode: "red_build", alt: "Build is broken", emoji: "❌" },
      { shortcode: "rollback", alt: "Rolling back the release", emoji: "⏪" },
      { shortcode: "merged", alt: "Pull request merged", emoji: "🟣" },
      { shortcode: "reviewing", alt: "Reviewing the diff", emoji: "🔍" },
    ],
  },
  {
    // Published but NOT in the catalog — drives the owner-only
    // "Awaiting catalog approval" queue in `StickerSettingsCard`.
    eventId: `d2${"d2".repeat(31)}`,
    author: BOB_PUBKEY,
    identifier: "night-owls",
    title: "Night Owls",
    description: "Submitted by Bob, waiting on a curator.",
    createdAt: SEED_CREATED_AT + 120,
    stickers: [
      { shortcode: "owl_hi", alt: "Owl saying hi", emoji: "🦉" },
      { shortcode: "owl_coding", alt: "Owl at a keyboard", emoji: "⌨️" },
      { shortcode: "owl_coffee", alt: "Owl refilling coffee", emoji: "☕" },
      { shortcode: "owl_sleep", alt: "Owl finally asleep", emoji: "🌙" },
    ],
  },
];

/** Catalog-approved packs, in catalog order. */
export const APPROVED_STICKER_PACK_SEEDS = MOCK_STICKER_PACK_SEEDS.slice(0, 2);

/** Deterministic id for the kind:13536 catalog snapshot. */
export const MOCK_STICKER_CATALOG_EVENT_ID = `e3${"e3".repeat(31)}`;

/** Deterministic id for the kind:10031 installed-packs list. */
export const MOCK_INSTALLED_STICKER_PACKS_EVENT_ID = `f4${"f4".repeat(31)}`;

export function stickerPackCoordinate(seed: MockStickerPackSeed): string {
  return `${KIND_STICKER_PACK}:${seed.author}:${seed.identifier}`;
}

/**
 * Every fixture asset, keyed by the cache-route path segment the UI requests
 * (`<author>/<identifier>/<shortcode>/<sha256>`). Specs use this to serve real
 * pixels for the exact URLs the app asks for.
 */
export function mockStickerAssetIndex(): Map<
  string,
  MockStickerSeed & { sha256: string; packTitle: string }
> {
  const index = new Map<
    string,
    MockStickerSeed & { sha256: string; packTitle: string }
  >();
  MOCK_STICKER_PACK_SEEDS.forEach((seed, slot) => {
    seed.stickers.forEach((sticker, position) => {
      const sha256 = assetHash(slot, position);
      index.set(
        `${seed.author}/${seed.identifier}/${sticker.shortcode}/${sha256}`,
        { ...sticker, sha256, packTitle: seed.title },
      );
    });
  });
  return index;
}

function mockEvent(
  id: string,
  kind: number,
  pubkey: string,
  createdAt: number,
  tags: string[][],
): RelayEvent {
  return {
    id,
    pubkey,
    created_at: createdAt,
    kind,
    tags,
    content: "",
    sig: "mocksig".repeat(20).slice(0, 128),
  };
}

/** Build the kind:30031 event for one seed. */
export function stickerPackEvent(
  seed: MockStickerPackSeed,
  slot: number,
): RelayEvent {
  const tags: string[][] = [
    ["d", seed.identifier],
    ["title", seed.title],
    ["pack_format", SONAR_PACK_FORMAT],
    ["t", SONAR_PACK_FORMAT],
    ["description", seed.description],
  ];
  if (seed.license) tags.push(["license", seed.license]);
  const coverHash = assetHash(slot, 0);
  tags.push([
    "image",
    stickerAssetUrl(
      seed.author,
      seed.identifier,
      seed.stickers[0].shortcode,
      coverHash,
    ),
    coverHash,
    "128x128",
  ]);
  seed.stickers.forEach((sticker, position) => {
    const sha256 = assetHash(slot, position);
    const url = stickerAssetUrl(
      seed.author,
      seed.identifier,
      sticker.shortcode,
      sha256,
    );
    tags.push([
      "sticker",
      sticker.shortcode,
      url,
      sha256,
      "image/webp",
      "128x128",
      sticker.alt,
      sticker.emoji,
    ]);
    // NIP-30 compatibility alias, exactly as `publishStickerPack` emits it.
    tags.push(["emoji", sticker.shortcode, url]);
  });
  return mockEvent(
    seed.eventId,
    KIND_STICKER_PACK,
    seed.author,
    seed.createdAt,
    tags,
  );
}

/** All kind:30031 pack events the mock relay serves. */
export function mockStickerPackEvents(): RelayEvent[] {
  return MOCK_STICKER_PACK_SEEDS.map(stickerPackEvent);
}

/**
 * The kind:13536 approved-catalog snapshot.
 *
 * The lone `["-"]` tag is mandatory: `catalogEntriesFromEvent` requires exactly
 * one of them and rejects the snapshot outright without it.
 */
export function mockStickerCatalogEvent(): RelayEvent {
  return mockEvent(
    MOCK_STICKER_CATALOG_EVENT_ID,
    13536,
    "f".repeat(64),
    SEED_CREATED_AT + 200,
    [
      ["-"],
      ...APPROVED_STICKER_PACK_SEEDS.map((seed) => [
        "a",
        stickerPackCoordinate(seed),
        seed.eventId,
      ]),
    ],
  );
}

/** The viewer's kind:10031 installed-pack list. */
export function mockInstalledStickerPacksEvent(): RelayEvent {
  return mockEvent(
    MOCK_INSTALLED_STICKER_PACKS_EVENT_ID,
    10031,
    MOCK_IDENTITY_PUBKEY,
    SEED_CREATED_AT + 240,
    APPROVED_STICKER_PACK_SEEDS.map((seed) => [
      "a",
      stickerPackCoordinate(seed),
    ]),
  );
}
