/**
 * Trims a Map to at most `maxSize` entries by deleting the oldest
 * (earliest-inserted) keys. Maps iterate in insertion order, so the
 * first keys are the oldest.
 */
export function trimMapToSize<K, V>(map: Map<K, V>, maxSize: number): void {
  if (map.size <= maxSize) return;
  const excess = map.size - maxSize;
  let removed = 0;
  for (const key of map.keys()) {
    if (removed >= excess) break;
    map.delete(key);
    removed++;
  }
}
