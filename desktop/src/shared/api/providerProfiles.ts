/** Credential metadata projected from the Rust provider catalog. */
export type AcpProviderCredential = {
  /** Environment variable used by headless deployments. No credential value is exposed. */
  env: string;
  label: string;
  deviceKeyring: boolean;
};

/** Non-secret provider facts owned by the Rust runtime catalog. */
export type AcpProviderProfile = {
  id: string;
  label: string;
  aliases: string[];
  modelEnv: string;
  baseUrlEnv: string | null;
  defaultBaseUrl: string;
  credential: AcpProviderCredential | null;
  requiredEnv: string[];
  supportsReasoningEffort: boolean;
};

/** Nested Tauri wire shape; Rust serializes provider profiles in camelCase. */
export type RawAcpProviderProfile = AcpProviderProfile;
