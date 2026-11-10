export type IdentityStorage =
  | "system-keyring"
  | "local-file"
  | "environment"
  | "ephemeral";

export type Identity = {
  pubkey: string;
  displayName: string;
  /** Durable location of the active identity key. Older/mock bridges may omit
   * this until they adopt identity storage reporting. */
  storage?: IdentityStorage;
  /** True when the app booted in "identity lost" recovery mode — the OS
   * keyring was empty despite a prior successful migration. The frontend
   * should route to nsec re-import instead of normal onboarding.
   * Mutually exclusive with `locked`. */
  lost?: boolean;
  /** True when the app booted with an ephemeral key because the OS keyring
   * holding the real identity is UNREACHABLE (e.g. GNOME Keyring / KWallet
   * locked). The real key still exists; no in-app recovery is possible —
   * the user must unlock the keyring externally and relaunch.
   * Mutually exclusive with `lost`. */
  locked?: boolean;
  /** True when the boot-time Phase 2 reset attempted a wipe but verification
   * failed. Identity resolution was skipped; the sentinel is preserved so
   * the next relaunch retries the wipe automatically. */
  resetFailed?: boolean;
};
