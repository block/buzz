/**
 * Registers every listener, tolerating individual failures — unlike
 * `Promise.all`, one rejected registration never loses track of siblings
 * that succeeded (including ones that only resolve *after* the rejection),
 * so the caller can still unlisten them instead of leaking native
 * subscriptions.
 */
export async function registerListeners<T>(
  registrations: Array<() => Promise<T>>,
): Promise<{ succeeded: T[]; failedReasons: unknown[] }> {
  const settled = await Promise.allSettled(
    registrations.map((register) => register()),
  );
  const succeeded: T[] = [];
  const failedReasons: unknown[] = [];
  for (const result of settled) {
    if (result.status === "fulfilled") {
      succeeded.push(result.value);
    } else {
      failedReasons.push(result.reason);
    }
  }
  return { succeeded, failedReasons };
}
