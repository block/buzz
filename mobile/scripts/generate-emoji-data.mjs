/**
 * Generate the mobile emoji dataset from emoji-mart.
 *
 * Desktop's emoji picker, autocomplete, and reaction display names all read
 * `@emoji-mart/data/sets/15/native.json` (see
 * `desktop/src/features/custom-emoji/ui/EmojiPicker.tsx` and
 * `desktop/src/shared/lib/emojiName.ts`). Mobile can't consume the npm package,
 * so this script projects the same source into a trimmed JSON asset that ships
 * with the Flutter app. Same ids, same names, same keywords, same category
 * order — so a `:shortcode:` means the same thing on both clients.
 *
 * The output is committed. Regenerate with `just mobile-emoji-data` after
 * bumping the emoji-mart dependency.
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const mobileRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(mobileRoot, "..");

const DATA_SUBPATH = "@emoji-mart/data/sets/15/native.json";
// `EMOJI_MART_DATA` lets a git worktree (which has no desktop/node_modules of
// its own) point at the main checkout's install instead of paying for a full
// `pnpm install` just to regenerate a committed asset.
const SOURCE =
  process.env.EMOJI_MART_DATA ??
  path.join(repoRoot, "desktop/node_modules", DATA_SUBPATH);
const OUTPUT = path.join(mobileRoot, "assets/emoji/emoji-data.json");

let raw;
try {
  raw = readFileSync(SOURCE, "utf8");
} catch {
  console.error(
    `Missing emoji-mart data at ${SOURCE}.\n` +
      "Run `pnpm install` in desktop/ first — this script projects the same\n" +
      "dataset the desktop picker uses so the two clients cannot drift.\n" +
      "From a worktree, point EMOJI_MART_DATA at the main checkout's copy:\n" +
      `  EMOJI_MART_DATA=<main-checkout>/desktop/node_modules/${DATA_SUBPATH}`,
  );
  process.exit(1);
}

const data = JSON.parse(raw);

// Category order is emoji-mart's own, which is the order desktop's picker
// renders. Preserve it verbatim rather than re-sorting.
const categories = data.categories.map((category) => ({
  id: category.id,
  emoji: category.emojis.filter((id) => {
    const entry = data.emojis[id];
    return Boolean(entry?.skins?.some((skin) => skin.native));
  }),
}));

// Keys are deliberately short (`n`/`u`/`k`) — the map is ~1.9k entries and the
// asset ships in the app bundle.
const emoji = {};
for (const category of categories) {
  for (const id of category.emoji) {
    const entry = data.emojis[id];
    emoji[id] = {
      n: entry.name,
      // Keep every skin variation. The app flattens these into selectable
      // tiles and maps every glyph back to this desktop-identical shortcode.
      u: entry.skins.map((skin) => skin.native).filter(Boolean),
      k: entry.keywords ?? [],
    };
  }
}

const payload = { categories, emoji };

mkdirSync(path.dirname(OUTPUT), { recursive: true });
// Trailing newline keeps the file diff-friendly; no indentation because the
// asset is machine-read only and indentation would roughly double its size.
writeFileSync(OUTPUT, `${JSON.stringify(payload)}\n`, "utf8");

console.log(
  `Wrote ${path.relative(repoRoot, OUTPUT)} — ` +
    `${categories.length} categories, ${Object.keys(emoji).length} emoji.`,
);
