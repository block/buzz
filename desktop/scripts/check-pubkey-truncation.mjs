import path from "node:path";
import { fileURLToPath } from "node:url";
import { runPubkeyTruncationCheck } from "../../scripts/check-pubkey-truncation-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

// Truncated pubkey prefixes are forgeable (vanity grinding), so all display
// truncation goes through the canonical `truncatePubkey` / `<PubKey>` — this
// guard keeps ad-hoc `pubkey.slice(0, N)` forms from fragmenting again.
const rules = [
  {
    root: "src",
    extensions: new Set([".ts", ".tsx"]),
  },
];

// Non-display uses: array windows over pubkey lists, color/initials
// derivation where the value is never presented as an identity.
const overrides = new Set([
  // ProfileAvatar fallback label — decorative glyphs inside an avatar disc.
  "src/features/huddle/components/ParticipantList.tsx:92",
  // HexAvatar: 6-char badge + hue derivation inside a color-coded disc,
  // clearly decorative (paired with a full truncatePubkey aria-label).
  "src/features/huddle/components/ParticipantList.tsx:143",
  "src/features/huddle/components/ParticipantList.tsx:144",
  // clientId (not a pubkey) sliced in a debug log next to the real thing.
  "src/features/channels/readState/readStateManager.ts:338",
  // Array windows (first N pubkeys), not string truncation.
  "src/features/messages/lib/threadPanel.ts:395",
  "src/features/projects/ui/ProjectsView.tsx:166",
  "src/features/projects/ui/ProjectsOverviewPanel.tsx:209",
]);

await runPubkeyTruncationCheck({
  projectRoot,
  rules,
  overrides,
  allowedFiles: new Set([
    // The canonical helper itself.
    "src/shared/lib/pubkey.ts",
    // E2E mock bridge fabricates ids/nsecs from pubkeys; nothing here is a
    // user-facing identity display.
    "src/testing/e2eBridge.ts",
  ]),
  label: "Desktop",
  scriptPath: "desktop/scripts/check-pubkey-truncation.mjs",
});
