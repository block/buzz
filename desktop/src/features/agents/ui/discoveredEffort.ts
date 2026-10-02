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
      if (
        item.value &&
        item.value !== "default" &&
        !values.includes(item.value)
      )
        values.push(item.value);
      if (item.options) visit(item.options);
    }
  }
  visit(option?.options);
  return values;
}
