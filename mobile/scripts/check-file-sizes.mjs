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

await runFileSizeCheck({
  projectRoot,
  rules,
  label: "Mobile",
});
