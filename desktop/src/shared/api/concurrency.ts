export async function collectWithConcurrency<T, R>(
  items: T[],
  concurrency: number,
  worker: (item: T) => Promise<R>,
): Promise<R[]> {
  const workerCount = Math.min(Math.max(1, concurrency), items.length);
  const results = new Array<R>(items.length);
  let nextIndex = 0;
  let firstError: unknown;
  let hasError = false;
  let stopped = false;

  await Promise.all(
    Array.from({ length: workerCount }, async () => {
      while (!stopped && nextIndex < items.length) {
        const currentIndex = nextIndex++;
        try {
          results[currentIndex] = await worker(items[currentIndex]);
        } catch (error) {
          if (!stopped) firstError = error;
          hasError = true;
          stopped = true;
        }
      }
    }),
  );

  if (hasError) throw firstError;
  return results;
}
