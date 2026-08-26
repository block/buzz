/** Right-edge context counts omit empty values, matching the Projects overview. */
export function presentContextCount(
  value: number | undefined,
): number | undefined {
  return value != null && value > 0 ? value : undefined;
}
