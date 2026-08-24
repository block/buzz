#!/usr/bin/env node

import { PiAcpRpcSpike } from "./adapter.mjs";

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
