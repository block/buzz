// Deterministic witness and owner keys shared between vitest configs
// (which bake pubkeys into bindings) and the pulse suites (which sign
// evidence with the secrets). Test-only material.

export const TEST_WITNESS_SECRET_HEX =
  "3333333333333333333333333333333333333333333333333333333333333333";

export const TEST_OWNER_SECRET_HEX =
  "4444444444444444444444444444444444444444444444444444444444444444";

export const TEST_PULSE_NODE_LABEL = "cf-rendezvous-test";
