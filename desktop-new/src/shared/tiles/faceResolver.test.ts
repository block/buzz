import { afterEach, describe, expect, it, vi } from "vitest";

import type { TileAddress } from "./address";
import { fallbackLabel, resetTileFaces, tileFaces } from "./faceResolver";

const MORGAN: TileAddress = { kind: "person", id: "pk-morgan" };
const FULL_KEY: TileAddress = {
  kind: "person",
  id: "9f2c4a1b7e5d8306a4b2c1d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e0f9",
};

afterEach(() => {
  resetTileFaces();
});

describe("tile face resolution", () => {
  it("always returns something renderable, so a tile is never faceless", () => {
    const face = tileFaces.get(MORGAN);
    expect(face.label.length).toBeGreaterThan(0);
  });

  /**
   * A face that shows a full identity is the defect the current client
   * shipped: a same-name collision there forces a 64-character key into
   * visible text. An unresolved tile abbreviates instead.
   */
  it("never exposes a full identity in a fallback label", () => {
    expect(fallbackLabel(FULL_KEY)).not.toContain(FULL_KEY.id);
    expect(fallbackLabel(FULL_KEY).length).toBeLessThan(20);
    expect(tileFaces.get(FULL_KEY).label).not.toContain(FULL_KEY.id);
  });

  it("resolves a name from a source without the caller knowing", () => {
    tileFaces.setSource({
      peek: (address) =>
        address.id === MORGAN.id
          ? { label: "Morgan", loading: false, resolved: true }
          : undefined,
    });

    expect(tileFaces.get(MORGAN).label).toBe("Morgan");
  });

  it("requests an unknown address once, however often it renders", () => {
    const request = vi.fn();
    tileFaces.setSource({ peek: () => undefined, request });

    tileFaces.get(MORGAN);
    tileFaces.get(MORGAN);
    tileFaces.get(MORGAN);

    expect(request).toHaveBeenCalledTimes(1);
  });

  /**
   * A rename must reach every tile without the document changing, because a
   * document change would dirty the person's draft and enter undo history.
   * Subscribers are how the face updates independently of the document.
   */
  it("notifies subscribers when a name changes", () => {
    const listener = vi.fn();
    const unsubscribe = tileFaces.subscribe(listener);

    tileFaces.put(MORGAN, { label: "Morgan", loading: false, resolved: true });
    expect(listener).toHaveBeenCalled();
    expect(tileFaces.get(MORGAN).label).toBe("Morgan");

    tileFaces.put(MORGAN, {
      label: "Morgan Mulvaney",
      loading: false,
      resolved: true,
    });
    expect(tileFaces.get(MORGAN).label).toBe("Morgan Mulvaney");

    unsubscribe();
  });

  it("stops notifying after unsubscribe", () => {
    const listener = vi.fn();
    tileFaces.subscribe(listener)();

    tileFaces.put(MORGAN, { label: "Morgan", loading: false, resolved: true });

    expect(listener).not.toHaveBeenCalled();
  });

  /**
   * Community-scoped cache. A name resolved in one community is not valid in
   * another, and a stale name surviving a switch is exactly the leak the
   * desktop client's reset inventory exists to prevent.
   */
  it("forgets every resolved name when the community resets", () => {
    tileFaces.put(MORGAN, { label: "Morgan", loading: false, resolved: true });
    expect(tileFaces.get(MORGAN).label).toBe("Morgan");

    resetTileFaces();

    expect(tileFaces.get(MORGAN).label).not.toBe("Morgan");
  });

  it("drops the previous community's source on reset", () => {
    tileFaces.setSource({
      peek: () => ({
        label: "Old community name",
        loading: false,
        resolved: true,
      }),
    });
    expect(tileFaces.get(MORGAN).label).toBe("Old community name");

    resetTileFaces();

    expect(tileFaces.get(MORGAN).label).not.toBe("Old community name");
  });
});

describe("assistive semantics depend on resolution", () => {
  it("marks an unresolved face as unresolved rather than naming it", () => {
    const face = tileFaces.get(FULL_KEY);
    expect(face.resolved).toBe(false);
  });

  it("marks a resolved face as resolved", () => {
    tileFaces.put(MORGAN, { label: "Morgan", loading: false, resolved: true });
    expect(tileFaces.get(MORGAN).resolved).toBe(true);
  });

  /**
   * A snapshot must be reference-stable between changes or React's
   * `useSyncExternalStore` re-renders forever and tears down the subtree.
   * That is not hypothetical: an earlier version built a fresh unresolved face
   * on every call and unmounted the whole composer.
   */
  it("returns a reference-stable face between changes", () => {
    const first = tileFaces.get(MORGAN);
    expect(tileFaces.get(MORGAN)).toBe(first);

    tileFaces.put(MORGAN, { label: "Morgan", loading: false, resolved: true });
    const second = tileFaces.get(MORGAN);
    expect(second).not.toBe(first);
    expect(tileFaces.get(MORGAN)).toBe(second);
  });
});
