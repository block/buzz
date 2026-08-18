import path from "node:path";
import { fileURLToPath } from "node:url";
import { runPubkeyTruncationCheck } from "../../scripts/check-pubkey-truncation-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

const rules = [
  {
    root: "src",
    extensions: new Set([".ts", ".tsx"]),
  },
];

const overrides = new Set([
  // Avatar fallback initials — two glyphs inside an avatar disc.
  "src/features/repos/ui/PubkeyAvatar.tsx:29",
  // Array window (first N pubkeys), not string truncation.
  "src/features/repos/ui/OrgSidebar.tsx:22",
]);

await runPubkeyTruncationCheck({
  projectRoot,
  rules,
  overrides,
  allowedFiles: new Set(["src/shared/lib/pubkey.ts"]),
  label: "Web",
  scriptPath: "web/scripts/check-pubkey-truncation.mjs",
});
