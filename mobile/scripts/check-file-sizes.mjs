import path from "node:path";
import { fileURLToPath } from "node:url";
import { runFileSizeCheck } from "../../scripts/check-file-sizes-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

const MAX_LINES = 1000;

const rules = [
  {
    root: "lib",
    extensions: new Set([".dart"]),
    maxLines: MAX_LINES,
  },
];

// TEMP — these files exceed the 1000-line limit and are queued to be split.
// Do not add to this list; split the file instead. Remove each entry as its
// file is broken up.
const overrides = new Map([
  ["lib/features/channels/channels_page.dart", 2423],
  ["lib/features/channels/channel_detail_page.dart", 1305],
  ["lib/features/channels/compose_bar.dart", 1111],
]);

await runFileSizeCheck({
  projectRoot,
  rules,
  overrides,
  label: "Mobile",
  scriptPath: "mobile/scripts/check-file-sizes.mjs",
});
