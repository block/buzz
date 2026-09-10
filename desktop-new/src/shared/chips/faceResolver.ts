import { type ChipAddress, type ChipKind, chipAddressKey } from "./address";

/**
 * What a chip shows. Derived from an address, never stored in the document.
 *
 * Keeping the face out of the document is the load-bearing decision: if a
 * display name lived in the draft, every profile rename would be an edit to
 * the person's unsent message — dirtying it and polluting undo history. So the
 * document holds the address and the face is looked up at render time.
 */
export type ChipFace = {
  /** The compact label a person reads. Never the identity. */
  label: string;
  avatarUrl?: string | null;
  /** Live status where the kind has one; absent means "no status to show". */
  status?: "online" | "busy" | "offline";
  /** True while the face is still being fetched, so a chip can show a skeleton. */
  loading: boolean;
  /**
   * Whether `label` is a real display name or a stand-in derived from the id.
   * Assistive semantics need this: announcing an abbreviated identity tells a
   * screen-reader user nothing, so an unresolved chip says so instead.
   */
  resolved: boolean;
};

/** The face shown before anything is known, and if a lookup finds nothing. */
export function unresolvedFace(address: ChipAddress): ChipFace {
  return { label: fallbackLabel(address), loading: false, resolved: false };
}

/**
 * A readable stand-in when an identity cannot be resolved.
 *
 * Deliberately an abbreviation of the id rather than a raw 64-character key: a
 * chip's face must never expose the full identity, because a face that shows a
 * key is the defect the current client shipped.
 */
export function fallbackLabel(address: ChipAddress): string {
  const { id } = address;
  if (id.length <= 12) return id;
  return `${id.slice(0, 8)}…${id.slice(-4)}`;
}

export type ChipFaceSource = {
  /** Synchronous read from whatever this surface already knows. */
  peek: (address: ChipAddress) => ChipFace | undefined;
  /** Kicks off a fetch for an address the cache does not hold yet. */
  request?: (address: ChipAddress) => void;
};

type Listener = () => void;

/**
 * Resolves chip faces from addresses, and tells subscribers when one changes.
 *
 * This is a module-level cache holding community-scoped data, so it must be
 * reset when the community changes or a name from the previous community leaks
 * into the new one. See `resetChipFaces`.
 */
class ChipFaceRegistry {
  private faces = new Map<string, ChipFace>();
  private listeners = new Set<Listener>();
  private source: ChipFaceSource | null = null;
  private requested = new Set<string>();

  setSource(source: ChipFaceSource | null): void {
    this.source = source;
    this.requested.clear();
    this.emit();
  }

  /**
   * The current face for an address. Always returns something renderable —
   * a chip never has "no face", it has an unresolved one.
   *
   * The returned object must be reference-stable between changes. React's
   * `useSyncExternalStore` compares snapshots by identity, so handing back a
   * fresh object each call is an infinite render loop that tears down the
   * whole subtree — which is exactly what a freshly-built unresolved face did
   * here. Unresolved faces are therefore cached like resolved ones.
   */
  get(address: ChipAddress): ChipFace {
    const key = chipAddressKey(address);
    const known = this.faces.get(key);
    if (known) return known;

    const peeked = this.source?.peek(address);
    if (peeked) {
      this.faces.set(key, peeked);
      return peeked;
    }

    // Ask once per address, and only if someone is actually rendering it.
    if (this.source?.request && !this.requested.has(key)) {
      this.requested.add(key);
      this.source.request(address);
    }

    const unresolved: ChipFace = {
      label: fallbackLabel(address),
      loading: Boolean(this.source),
      resolved: false,
    };
    this.faces.set(key, unresolved);
    return unresolved;
  }

  /** Records a resolved face and notifies anything rendering that address. */
  put(address: ChipAddress, face: ChipFace): void {
    this.faces.set(chipAddressKey(address), face);
    this.emit();
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  reset(): void {
    this.faces.clear();
    this.requested.clear();
    this.source = null;
    this.emit();
  }

  private emit(): void {
    for (const listener of this.listeners) listener();
  }
}

export const chipFaces = new ChipFaceRegistry();

/**
 * Clears every resolved face.
 *
 * Must be called when the community changes. A face resolved in one community
 * is not valid in another, and a stale name surviving a switch is the exact
 * leak the desktop client's `resetCommunityState()` inventory exists to
 * prevent.
 */
export function resetChipFaces(): void {
  chipFaces.reset();
}

/**
 * The sigil a kind is introduced by, independent of any lookup.
 *
 * A message and a link have no sigil a reader would recognise, so they carry an
 * icon instead and prefix nothing.
 */
export const CHIP_KIND_TRIGGER: Record<ChipKind, string> = {
  person: "@",
  agent: "@",
  channel: "#",
  message: "",
  link: "",
};
