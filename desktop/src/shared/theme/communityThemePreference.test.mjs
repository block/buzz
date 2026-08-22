import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  DEFAULT_COMMUNITY_THEME,
  cacheAndApplyCommunityTheme,
  captureCommunityThemeAppearanceSnapshot,
  clearCommunityThemeOutbox,
  communityThemeAppearanceFallback,
  communityThemeApplyExpectation,
  communityThemeMigrationOutboxKey,
  communityThemeOutboxKey,
  communityThemePersistenceAction,
  communityThemeScopeFallback,
  communityThemeStorageKey,
  parseCommunityThemePreference,
  readCommunityThemeAppearanceSnapshot,
  readCommunityThemeCurrentAppearance,
  readCommunityThemeMigrationOutbox,
  readCommunityThemeOutbox,
  readCommunityThemePreference,
  refreshCommunityThemeCurrentAppearance,
  sameCommunityThemePreference,
  writeCommunityThemeMigrationOutbox,
  writeCommunityThemeOutbox,
  writeCommunityThemePreference,
} from "./communityThemePreference.ts";
import {
  ACCENT_COLORS,
  GLASS_OPACITY_MAX,
  GLASS_OPACITY_MIN,
} from "./ThemeProvider.tsx";
import { SYNTAX_THEMES } from "./theme-loader.ts";

function localStorageStub() {
  const data = new Map();
  return {
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => data.set(key, String(value)),
    removeItem: (key) => data.delete(key),
  };
}

test("parses only the versioned stable appearance contract", () => {
  const valid = {
    version: 1,
    theme: "houston",
    accent: "#a855f7",
    followSystem: false,
    glassBackground: true,
    glassOpacity: 47,
    prominentActiveTab: false,
  };
  assert.deepEqual(parseCommunityThemePreference(valid), valid);
  assert.equal(parseCommunityThemePreference({ ...valid, version: 2 }), null);
  assert.equal(
    parseCommunityThemePreference({ ...valid, theme: "future-theme" }),
    null,
  );
  assert.equal(
    parseCommunityThemePreference({ ...valid, accent: "url(image)" }),
    null,
  );
  assert.equal(
    parseCommunityThemePreference({ ...valid, followSystem: "false" }),
    null,
  );
  assert.equal(
    parseCommunityThemePreference({ ...valid, glassBackground: "true" }),
    null,
  );
  assert.equal(
    parseCommunityThemePreference({ ...valid, glassOpacity: 29 }),
    null,
  );
  assert.equal(
    parseCommunityThemePreference({ ...valid, prominentActiveTab: 1 }),
    null,
  );
  assert.equal(
    parseCommunityThemePreference({ ...valid, glassBackground: null }),
    null,
  );
  assert.equal(
    parseCommunityThemePreference({ ...valid, glassOpacity: null }),
    null,
  );
  assert.equal(
    parseCommunityThemePreference({ ...valid, prominentActiveTab: null }),
    null,
  );
});

test("older theme records inherit the pre-migration appearance controls", () => {
  const legacy = {
    version: 1,
    theme: "houston",
    accent: "#a855f7",
    followSystem: false,
  };
  const fallback = {
    ...DEFAULT_COMMUNITY_THEME,
    glassBackground: true,
    glassOpacity: 42,
    prominentActiveTab: false,
  };

  assert.deepEqual(parseCommunityThemePreference(legacy, fallback), {
    ...legacy,
    glassBackground: true,
    glassOpacity: 42,
    prominentActiveTab: false,
  });
});

test("appearance fallback uses the durable snapshot, else stable defaults", () => {
  const olderRecord = {
    version: 1,
    theme: "houston",
    accent: "#a855f7",
    followSystem: false,
  };

  // No snapshot: an upgrading record resolves to the stable defaults.
  assert.deepEqual(
    parseCommunityThemePreference(
      olderRecord,
      communityThemeAppearanceFallback(null),
    ),
    { ...DEFAULT_COMMUNITY_THEME, ...olderRecord },
  );

  // With a snapshot: the record inherits the profile's pre-migration glass
  // and prominent-tab choices, never a previous community's live appearance.
  const snapshot = {
    glassBackground: true,
    glassOpacity: 42,
    prominentActiveTab: true,
  };
  assert.deepEqual(
    parseCommunityThemePreference(
      olderRecord,
      communityThemeAppearanceFallback(snapshot),
    ),
    { ...olderRecord, ...snapshot },
  );
});

test("appearance snapshot is captured once and consumed per community", () => {
  globalThis.window = { localStorage: localStorageStub() };
  const inherited = {
    ...DEFAULT_COMMUNITY_THEME,
    glassBackground: true,
    glassOpacity: 80,
    prominentActiveTab: true,
  };
  const legacy = {
    version: 1,
    theme: "houston",
    accent: "#a855f7",
    followSystem: false,
  };
  window.localStorage.setItem(
    communityThemeStorageKey("alice", "wss://a.example"),
    JSON.stringify(legacy),
  );
  window.localStorage.setItem(
    communityThemeStorageKey("alice", "wss://b.example"),
    JSON.stringify(legacy),
  );

  // Snapshot is empty until first captured.
  assert.equal(readCommunityThemeAppearanceSnapshot("alice"), null);
  const snapshot = captureCommunityThemeAppearanceSnapshot("alice", inherited);
  assert.deepEqual(snapshot, {
    glassBackground: true,
    glassOpacity: 80,
    prominentActiveTab: true,
  });

  // A later community switch drifts the live appearance to defaults, but the
  // snapshot stays pinned so every legacy record inherits the same values.
  const drifted = {
    ...DEFAULT_COMMUNITY_THEME,
    glassBackground: false,
    glassOpacity: 65,
    prominentActiveTab: false,
  };
  assert.deepEqual(
    captureCommunityThemeAppearanceSnapshot("alice", drifted),
    snapshot,
  );

  const expected = { ...legacy, ...snapshot };
  const fallback = communityThemeAppearanceFallback(
    readCommunityThemeAppearanceSnapshot("alice"),
  );
  assert.deepEqual(
    readCommunityThemePreference("alice", "wss://a.example", fallback),
    expected,
  );
  assert.deepEqual(
    readCommunityThemePreference("alice", "wss://b.example", fallback),
    expected,
  );
});

test("a full store still yields the correct in-session appearance fallback", () => {
  // The controller captures the snapshot once, then reuses the returned value
  // across its effects via a ref. This pins the contract that reuse depends on:
  // even when the snapshot write is rejected (a full store), capture returns
  // the live pre-migration appearance, so a legacy record still inherits it.
  globalThis.window = {
    localStorage: {
      getItem: () => null,
      setItem: () => {
        throw new Error("quota exceeded");
      },
      removeItem: () => {},
    },
  };
  const inherited = {
    ...DEFAULT_COMMUNITY_THEME,
    glassBackground: true,
    glassOpacity: 80,
    prominentActiveTab: true,
  };
  const snapshot = captureCommunityThemeAppearanceSnapshot("alice", inherited);
  assert.deepEqual(snapshot, {
    glassBackground: true,
    glassOpacity: 80,
    prominentActiveTab: true,
  });
  // A re-read from storage collapses to defaults, which is exactly why the
  // controller must reuse the captured value rather than re-reading it.
  assert.equal(readCommunityThemeAppearanceSnapshot("alice"), null);
  const legacy = {
    version: 1,
    theme: "houston",
    accent: "#a855f7",
    followSystem: false,
  };
  const fallback = communityThemeAppearanceFallback(snapshot);
  assert.deepEqual(parseCommunityThemePreference(legacy, fallback), {
    ...legacy,
    ...snapshot,
  });
});

test("a full store pins the snapshot across a controller remount", () => {
  // The controller is remounted per community under a keyed provider, so a
  // ref cannot carry the snapshot between mounts. When the store is full the
  // capture must still return the profile's first-seen value on the next
  // mount, rather than re-capturing the current (previous community's)
  // appearance. This pins that the in-memory fallback has profile lifetime.
  globalThis.window = {
    localStorage: {
      getItem: () => null,
      setItem: () => {
        throw new Error("quota exceeded");
      },
      removeItem: () => {},
    },
  };
  const firstMount = {
    ...DEFAULT_COMMUNITY_THEME,
    glassBackground: true,
    glassOpacity: 80,
    prominentActiveTab: true,
  };
  const first = captureCommunityThemeAppearanceSnapshot(
    "remount-pk",
    firstMount,
  );
  assert.deepEqual(first, {
    glassBackground: true,
    glassOpacity: 80,
    prominentActiveTab: true,
  });

  // The next mount is a different community whose applied appearance differs.
  // Capture must return the pinned first value, not this community's.
  const secondMount = {
    ...DEFAULT_COMMUNITY_THEME,
    glassBackground: false,
    glassOpacity: 30,
    prominentActiveTab: false,
  };
  assert.deepEqual(
    captureCommunityThemeAppearanceSnapshot("remount-pk", secondMount),
    first,
  );
});

test("a user glass edit updates absent scopes without changing the legacy snapshot", () => {
  // Empty communities track the latest explicit choice, but a legacy record
  // must always inherit the immutable pre-migration appearance.
  globalThis.window = { localStorage: localStorageStub() };
  const preMigration = {
    ...DEFAULT_COMMUNITY_THEME,
    glassBackground: true,
    glassOpacity: 80,
    prominentActiveTab: true,
  };
  const pinned = captureCommunityThemeAppearanceSnapshot(
    "edit-pk",
    preMigration,
  );

  const edited = {
    ...DEFAULT_COMMUNITY_THEME,
    glassBackground: false,
    glassOpacity: 40,
    prominentActiveTab: false,
  };
  const current = refreshCommunityThemeCurrentAppearance("edit-pk", edited);
  assert.deepEqual(current, {
    glassBackground: false,
    glassOpacity: 40,
    prominentActiveTab: false,
  });
  assert.deepEqual(
    readCommunityThemeCurrentAppearance("edit-pk", pinned),
    current,
  );

  // The next mount still sees the original snapshot for an older payload.
  assert.deepEqual(
    captureCommunityThemeAppearanceSnapshot("edit-pk", edited),
    pinned,
  );
  const legacy = {
    version: 1,
    theme: "houston",
    accent: "#a855f7",
    followSystem: false,
  };
  assert.deepEqual(
    parseCommunityThemePreference(
      legacy,
      communityThemeAppearanceFallback(pinned),
    ),
    { ...legacy, ...pinned },
  );
});

test("desktop appearance limits match the shared wire contract", () => {
  const contract = JSON.parse(
    readFileSync(
      new URL("../../../../schema/community-theme-v1.json", import.meta.url),
      "utf8",
    ),
  );

  assert.equal(
    DEFAULT_COMMUNITY_THEME.glassBackground,
    contract.properties.glassBackground.default,
  );
  assert.equal(
    DEFAULT_COMMUNITY_THEME.glassOpacity,
    contract.properties.glassOpacity.default,
  );
  assert.equal(GLASS_OPACITY_MIN, contract.properties.glassOpacity.minimum);
  assert.equal(GLASS_OPACITY_MAX, contract.properties.glassOpacity.maximum);
  assert.equal(
    DEFAULT_COMMUNITY_THEME.prominentActiveTab,
    contract.properties.prominentActiveTab.default,
  );
  assert.deepEqual(
    [...contract.properties.theme.enum].sort(),
    [...SYNTAX_THEMES].sort(),
  );
  assert.deepEqual(
    [...contract.properties.accent.enum].sort(),
    ACCENT_COLORS.map(({ value }) => value).sort(),
  );
});

test("appearance equality includes glass and prominent-tab choices", () => {
  assert.equal(
    sameCommunityThemePreference(DEFAULT_COMMUNITY_THEME, {
      ...DEFAULT_COMMUNITY_THEME,
      glassOpacity: DEFAULT_COMMUNITY_THEME.glassOpacity + 1,
    }),
    false,
  );
  assert.equal(
    sameCommunityThemePreference(DEFAULT_COMMUNITY_THEME, {
      ...DEFAULT_COMMUNITY_THEME,
      glassBackground: !DEFAULT_COMMUNITY_THEME.glassBackground,
    }),
    false,
  );
  assert.equal(
    sameCommunityThemePreference(DEFAULT_COMMUNITY_THEME, {
      ...DEFAULT_COMMUNITY_THEME,
      prominentActiveTab: !DEFAULT_COMMUNITY_THEME.prominentActiveTab,
    }),
    false,
  );
});

test("local preferences are isolated by pubkey and normalized relay", () => {
  globalThis.window = { localStorage: localStorageStub() };
  const aliceA = {
    ...DEFAULT_COMMUNITY_THEME,
    theme: "houston",
    followSystem: false,
  };
  const aliceB = { ...DEFAULT_COMMUNITY_THEME, theme: "catppuccin-latte" };
  const bobA = { ...DEFAULT_COMMUNITY_THEME, accent: "#ef4444" };
  assert.equal(
    writeCommunityThemePreference("alice", "WSS://A.EXAMPLE/", aliceA),
    true,
  );
  assert.equal(
    writeCommunityThemePreference("alice", "wss://b.example", aliceB),
    true,
  );
  assert.equal(
    writeCommunityThemePreference("bob", "wss://a.example", bobA),
    true,
  );
  assert.deepEqual(
    readCommunityThemePreference("alice", "wss://a.example"),
    aliceA,
  );
  assert.deepEqual(
    readCommunityThemePreference("alice", "wss://b.example/"),
    aliceB,
  );
  assert.deepEqual(
    readCommunityThemePreference("bob", "wss://a.example"),
    bobA,
  );
  assert.notEqual(
    communityThemeStorageKey("alice", "wss://a.example"),
    communityThemeStorageKey("alice", "wss://b.example"),
  );
});

test("dirty outbox survives restart and clears only its exact revision", () => {
  globalThis.window = { localStorage: localStorageStub() };
  const first = { ...DEFAULT_COMMUNITY_THEME, theme: "houston" };
  const second = { ...DEFAULT_COMMUNITY_THEME, accent: "#ef4444" };

  assert.equal(
    writeCommunityThemeOutbox("alice", "WSS://A.EXAMPLE/", first),
    true,
  );
  assert.deepEqual(readCommunityThemeOutbox("alice", "wss://a.example"), first);
  writeCommunityThemeOutbox("alice", "wss://a.example", second);
  clearCommunityThemeOutbox("alice", "wss://a.example", first);
  assert.deepEqual(
    readCommunityThemeOutbox("alice", "wss://a.example"),
    second,
  );
  clearCommunityThemeOutbox("alice", "wss://a.example", second);
  assert.equal(readCommunityThemeOutbox("alice", "wss://a.example"), null);
  assert.notEqual(
    communityThemeOutboxKey("alice", "wss://a.example"),
    communityThemeStorageKey("alice", "wss://a.example"),
  );
});

test("migration upgrades are isolated from user edits", () => {
  globalThis.window = { localStorage: localStorageStub() };
  const migration = { ...DEFAULT_COMMUNITY_THEME, theme: "houston" };
  const userEdit = { ...DEFAULT_COMMUNITY_THEME, accent: "#ef4444" };

  assert.equal(
    writeCommunityThemeMigrationOutbox("alice", "wss://a.example", migration),
    true,
  );
  assert.deepEqual(
    readCommunityThemeMigrationOutbox("alice", "wss://a.example"),
    migration,
  );
  assert.equal(readCommunityThemeOutbox("alice", "wss://a.example"), null);

  // A genuine edit supersedes and clears the migration-only upgrade.
  assert.equal(
    writeCommunityThemeOutbox("alice", "wss://a.example", userEdit),
    true,
  );
  assert.equal(
    readCommunityThemeMigrationOutbox("alice", "wss://a.example"),
    null,
  );
  assert.deepEqual(
    readCommunityThemeOutbox("alice", "wss://a.example"),
    userEdit,
  );
  assert.notEqual(
    communityThemeMigrationOutboxKey("alice", "wss://a.example"),
    communityThemeOutboxKey("alice", "wss://a.example"),
  );
});

test("malformed local data returns null so switching can apply the safe default", () => {
  globalThis.window = { localStorage: localStorageStub() };
  const key = communityThemeStorageKey("alice", "wss://broken.example");
  window.localStorage.setItem(
    key,
    JSON.stringify({ version: 1, theme: "missing" }),
  );
  assert.equal(
    readCommunityThemePreference("alice", "wss://broken.example"),
    null,
  );
  window.localStorage.setItem(key, "{");
  assert.equal(
    readCommunityThemePreference("alice", "wss://broken.example"),
    null,
  );
});

test("remote preference still applies when its local cache write fails", () => {
  globalThis.window = {
    localStorage: {
      getItem: () => null,
      setItem: () => {
        throw new Error("quota exceeded");
      },
    },
  };
  let applied = null;
  cacheAndApplyCommunityTheme(
    "alice",
    "wss://a.example",
    DEFAULT_COMMUNITY_THEME,
    (preference) => {
      applied = preference;
    },
  );
  assert.deepEqual(applied, DEFAULT_COMMUNITY_THEME);
});

test("already-applied relay state leaves the next user edit publishable", () => {
  const applied = {
    ...DEFAULT_COMMUNITY_THEME,
    theme: "catppuccin-latte",
    followSystem: false,
  };

  assert.equal(communityThemeApplyExpectation(applied, applied), null);
  assert.deepEqual(
    communityThemeApplyExpectation(applied, DEFAULT_COMMUNITY_THEME),
    applied,
  );
});

test("no-op initialization remains programmatic", () => {
  const expectation = communityThemeApplyExpectation(
    DEFAULT_COMMUNITY_THEME,
    DEFAULT_COMMUNITY_THEME,
    true,
  );

  assert.equal(
    communityThemePersistenceAction(expectation, DEFAULT_COMMUNITY_THEME),
    "acknowledge",
  );
});

test("confirmed first-community migration isolates later empty scopes", () => {
  const inherited = {
    ...DEFAULT_COMMUNITY_THEME,
    theme: "dracula",
    followSystem: false,
  };

  assert.deepEqual(communityThemeScopeFallback(false, inherited), inherited);
  assert.deepEqual(
    communityThemeScopeFallback(true, inherited),
    DEFAULT_COMMUNITY_THEME,
  );
});

test("community switch defers stale outgoing appearance persistence", () => {
  const outgoing = {
    ...DEFAULT_COMMUNITY_THEME,
    theme: "houston",
    followSystem: false,
  };
  const incoming = {
    ...DEFAULT_COMMUNITY_THEME,
    theme: "catppuccin-latte",
  };

  assert.equal(communityThemePersistenceAction(incoming, outgoing), "defer");
  assert.equal(
    communityThemePersistenceAction(incoming, incoming),
    "acknowledge",
  );
  assert.equal(communityThemePersistenceAction(null, incoming), "persist");
});
