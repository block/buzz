import type { RelayEvent } from "@/shared/api/types";

const AUTHOR =
  "e5ebc6cdb579be112e336cc319b5989b4bb6af11786ea90dbe52b5f08d741b34";
const NAMES = [
  ...Array.from({ length: 60 }, (_, i) => `bufo_wave_${i}`),
  "party_parrot",
  "bufo_party",
];

/** Kind 30180 entries the mock relay catalog serves; none are in the legacy union. */
export const MOCK_EMOJI_ENTRIES: RelayEvent[] = NAMES.map((name, i) => ({
  id: i.toString(16).padStart(64, "0"),
  pubkey: AUTHOR,
  created_at: 1_700_000_000 + i,
  kind: 30180,
  content: name.replace(/[_-]/g, " "),
  tags: [
    ["d", name],
    ["emoji", name, `https://example.com/e2e/${name}.svg`],
  ],
  sig: "0".repeat(128),
}));

/** Mock of the `search_emoji_entries` command: every query word prefixes a content word, at most `limit`. */
export function mockSearchEmojiEntries(input: {
  query: string;
  limit?: number;
}): RelayEvent[] {
  const queryWords = input.query.trim().toLowerCase().split(/\s+/);
  if (queryWords[0] === "") throw new Error("emoji search query is empty");
  return MOCK_EMOJI_ENTRIES.filter((event) => {
    const words = event.content.split(" ");
    return queryWords.every((q) => words.some((word) => word.startsWith(q)));
  }).slice(0, input.limit ?? 50);
}
