// Worker secrets are deployed with `wrangler secret put` and are invisible
// to `wrangler types`, so they are declared here instead of in the
// generated worker-configuration.d.ts (which must stay check-clean).
interface Env {
  /** 32-byte hex witness signing key; absent disables the Beacon pulse. */
  BUZZ_NODE_SECRET?: string;
}
