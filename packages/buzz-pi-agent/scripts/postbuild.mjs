import { chmod } from "node:fs/promises";
import { fileURLToPath } from "node:url";

await chmod(fileURLToPath(new URL("../dist/cli.js", import.meta.url)), 0o755);
