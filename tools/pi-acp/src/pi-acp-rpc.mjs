#!/usr/bin/env node

import { PiAcpRpcSpike } from "./adapter.mjs";

if (process.argv.includes("--version") || process.argv.includes("-V")) {
  process.stdout.write("pi-acp 0.1.0\n");
  process.exit(0);
}

const adapter = new PiAcpRpcSpike({
  input: process.stdin,
  output: process.stdout,
  errorOutput: process.stderr,
});
adapter.start();

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, async () => {
    await adapter.shutdown();
    process.exit(0);
  });
}
