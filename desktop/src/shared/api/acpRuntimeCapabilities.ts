export type AcpRuntimeCapabilityFacts = {
  /** Buzz-owned static fact from KnownAcpRuntime; null for older backends. */
  supportsAcpNativeConfig: boolean | null;
  /** Buzz-owned static fact from KnownAcpRuntime; null for older backends. */
  supportsAcpModelSwitching: boolean | null;
  /** Buzz-owned static fact from KnownAcpRuntime; null for older backends. */
  mcpHooks: boolean | null;
};
