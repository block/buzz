/** Joins optional class names without rewriting semantic size roles. */
export function cx(...values: Array<string | false | undefined>) {
  return values.filter(Boolean).join(" ");
}
/** Preserves Base UI state-aware className callbacks when adding the Buzz skin. */
export function skin<State>(
  base: string,
  custom?: string | ((state: State) => string | undefined),
) {
  return (state: State) =>
    cx(base, typeof custom === "function" ? custom(state) : custom);
}
