/**
 * Universal fallback pool of short, distinctive names used when a persona's
 * own name pool is empty or exhausted.
 */
const UNIVERSAL_POOL = [
  "Alder",
  "Brook",
  "Coral",
  "Dawn",
  "Echo",
  "Frost",
  "Gale",
  "Heath",
  "Ivy",
  "Jade",
  "Kite",
  "Luna",
  "Maple",
  "Nova",
  "Opal",
  "Pyre",
  "Quartz",
  "Rune",
  "Silk",
  "Thorn",
  "Umber",
  "Vale",
  "Wisp",
  "Yarn",
  "Zinc",
  "Brine",
  "Cove",
  "Drift",
  "Elm",
  "Fjord",
];

/**
 * Pick a random unused name for a bot instance.
 *
 * 1. Try the persona's own name pool first.
 * 2. If exhausted, fall back to the universal pool.
 * 3. If both are exhausted, append a 2-digit suffix to a random name.
 */
export function pickBotName(
  namePool: string[],
  usedNames: Set<string>,
): string {
  const usedLower = new Set([...usedNames].map((n) => n.toLowerCase()));

  const pick = (pool: readonly string[]) => {
    const available = pool.filter((n) => !usedLower.has(n.toLowerCase()));
    if (available.length === 0) return null;
    return available[Math.floor(Math.random() * available.length)];
  };

  if (namePool.length > 0) {
    const name = pick(namePool);
    if (name) return name;
  }

  const fallback = pick(UNIVERSAL_POOL);
  if (fallback) return fallback;

  const allNames = namePool.length > 0 ? namePool : UNIVERSAL_POOL;
  const base = allNames[Math.floor(Math.random() * allNames.length)];
  for (let i = 2; i < 100; i++) {
    const suffixed = `${base}-${String(i).padStart(2, "0")}`;
    if (!usedLower.has(suffixed.toLowerCase())) {
      return suffixed;
    }
  }

  return `${base}-${Date.now() % 1000}`;
}
