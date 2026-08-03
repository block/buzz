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
    if (!existingKeys.includes(key) || value.length > 0) {
      envEntries.set(key, value);
    }
  }
  const env = Object.fromEntries(envEntries);
  return {
    ok: true,
    env,
    removeEnvKeys: removedKeys.filter((key) => !(key in env)),
  };
}
