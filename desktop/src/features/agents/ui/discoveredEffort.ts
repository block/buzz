import type { AgentModelsResponse } from "@/shared/api/types";

/** Flatten only adapter-advertised values; an absent descriptor is not a static vocabulary. */
export function discoveredEffortValues(
  option: AgentModelsResponse["effortOption"],
): string[] {
  const values: string[] = [];
  function visit(
    options: NonNullable<AgentModelsResponse["effortOption"]>["options"],
  ) {
    for (const item of options ?? []) {
      if (item.value && !values.includes(item.value)) values.push(item.value);
      if (item.options) visit(item.options);
    }
  }
  visit(option?.options);
  return values;
}

/** An explicit effort edit replaces native and legacy values atomically. */
export function withEffortValue(
  env: Record<string, string>,
  keys: readonly string[],
  value = "",
): Record<string, string> {
  const next = Object.fromEntries(
    Object.entries(env).filter(
      ([key]) =>
        !keys.some((owned) => owned.toLowerCase() === key.toLowerCase()),
    ),
  );
  if (value && keys[0]) next[keys[0]] = value;
  return next;
}
