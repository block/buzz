import type { EnvVarsValue } from "./EnvVarsEditor";

export function hasText(value: string | null | undefined): boolean {
  return (value?.trim().length ?? 0) > 0;
}

export function hasAdvancedEnvVars(
  value: EnvVarsValue,
  managedKey: string | null,
): boolean {
  return Object.keys(value).some((key) => key !== managedKey);
}

export function getAdvancedEnvVars(
  value: EnvVarsValue,
  managedKey: string | null,
): EnvVarsValue {
  if (!managedKey || !(managedKey in value)) {
    return value;
  }

  const next = { ...value };
  delete next[managedKey];
  return next;
}
