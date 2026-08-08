export type ProjectConnectionSecretRow = {
  key: string;
  value: string;
};

export type ProjectConnectionSecretChanges =
  | {
      ok: true;
      env: Record<string, string>;
      removeEnvKeys: string[];
    }
  | { ok: false; error: string };

const MAX_SECRET_KEYS = 128;
const MAX_SECRET_BYTES = 64 * 1024;
const RESERVED_SECRET_KEYS = new Set([
  "BUZZ_PRIVATE_KEY",
  "NOSTR_PRIVATE_KEY",
  "BUZZ_AUTH_TAG",
  "BUZZ_API_TOKEN",
  "BUZZ_ACP_PRIVATE_KEY",
  "BUZZ_ACP_API_TOKEN",
  "BUZZ_RELAY_URL",
  "BUZZ_ACP_AGENT_COMMAND",
  "BUZZ_ACP_AGENT_ARGS",
  "BUZZ_ACP_MCP_COMMAND",
  "BUZZ_ACP_RESPOND_TO",
  "BUZZ_ACP_RESPOND_TO_ALLOWLIST",
  "BUZZ_ACP_AGENT_OWNER",
  "BUZZ_ACP_DISPLAY_NAME",
  "BUZZ_ACP_EXIT_AFTER_INACTIVITY",
  "BUZZ_ACP_NO_PRESENCE",
  "BUZZ_ACP_SETUP_PAYLOAD",
  "BUZZ_MANAGED_AGENT",
  "BUZZ_MANAGED_AGENT_START_NONCE",
]);

function byteLength(value: string) {
  return new TextEncoder().encode(value).byteLength;
}

export function buildProjectConnectionSecretChanges(
  rows: readonly ProjectConnectionSecretRow[],
  existingKeys: readonly string[],
  removedKeys: readonly string[],
): ProjectConnectionSecretChanges {
  const envEntries = new Map<string, string>();
  const seen = new Set<string>();
  for (const row of rows) {
    const key = row.key.trim();
    const value = row.value;
    if (!key && !value) continue;
    if (!/^[A-Z_][A-Z0-9_]*$/.test(key)) {
      return {
        ok: false,
        error:
          "Secret names can use uppercase letters, numbers, and underscores.",
      };
    }
    if (RESERVED_SECRET_KEYS.has(key)) {
      return {
        ok: false,
        error: `${key} is managed by Buzz and cannot be used here.`,
      };
    }
    if (seen.has(key)) {
      return {
        ok: false,
        error: "Each secret name can only appear once.",
      };
    }
    seen.add(key);
    if (!existingKeys.includes(key) && value.length === 0) {
      return {
        ok: false,
        error: `Enter a value for ${key}.`,
      };
    }
    if (value.includes("\0")) {
      return {
        ok: false,
        error: `Remove the invalid character from ${key}.`,
      };
    }
    if (!existingKeys.includes(key) || value.length > 0) {
      envEntries.set(key, value);
    }
  }
  const env = Object.fromEntries(envEntries);
  const finalKeys = new Set(
    existingKeys.filter((key) => !removedKeys.includes(key)),
  );
  for (const key of seen) finalKeys.add(key);
  if (finalKeys.size > MAX_SECRET_KEYS) {
    return {
      ok: false,
      error: `Use no more than ${MAX_SECRET_KEYS} secrets for one connection.`,
    };
  }
  let knownBytes = 0;
  for (const key of finalKeys) knownBytes += byteLength(key);
  for (const value of envEntries.values()) knownBytes += byteLength(value);
  if (knownBytes > MAX_SECRET_BYTES) {
    return {
      ok: false,
      error: "Keep the connection's secret values within 64 KiB.",
    };
  }
  return {
    ok: true,
    env,
    removeEnvKeys: removedKeys.filter((key) => !Object.hasOwn(env, key)),
  };
}
