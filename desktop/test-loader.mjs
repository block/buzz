import { registerHooks } from "node:module";
import { load, resolve } from "./test-loader-hooks.mjs";

registerHooks({ load, resolve });
