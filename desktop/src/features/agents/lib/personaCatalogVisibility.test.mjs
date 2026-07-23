import assert from "node:assert/strict";
import test from "node:test";

import {
  readCatalogPersonaMemoryLevels,
  readPublishedCatalogPersonaVersions,
  readSharedCatalogPersonaIds,
  writeCatalogPersonaMemoryLevels,
  writePublishedCatalogPersonaVersions,
  writeSharedCatalogPersonaIds,
} from "./personaCatalogVisibility.ts";

test("catalog visibility reads stored persona ids", () => {
  const storage = {
    getItem: () => JSON.stringify(["custom:analyst", 42, "custom:writer"]),
  };

  assert.deepEqual(readSharedCatalogPersonaIds(storage), [
    "custom:analyst",
    "custom:writer",
  ]);
});

test("catalog visibility tolerates unavailable and invalid storage", () => {
  assert.deepEqual(readSharedCatalogPersonaIds(null), []);
  assert.deepEqual(
    readSharedCatalogPersonaIds({ getItem: () => "not-json" }),
    [],
  );
  assert.deepEqual(readSharedCatalogPersonaIds({ getItem: () => "{}" }), []);
  assert.deepEqual(
    readSharedCatalogPersonaIds({
      getItem: () => {
        throw new Error("unavailable");
      },
    }),
    [],
  );
});

test("catalog visibility persists persona ids without blocking on storage errors", () => {
  let storedKey = "";
  let storedValue = "";
  writeSharedCatalogPersonaIds(["custom:analyst"], {
    setItem: (key, value) => {
      storedKey = key;
      storedValue = value;
    },
  });

  assert.equal(storedKey, "buzz-persona-catalog-visibility-v1");
  assert.equal(storedValue, '["custom:analyst"]');
  assert.doesNotThrow(() =>
    writeSharedCatalogPersonaIds(["custom:analyst"], {
      setItem: () => {
        throw new Error("unavailable");
      },
    }),
  );
});

test("catalog memory levels read only supported snapshot levels", () => {
  const storage = {
    getItem: () =>
      JSON.stringify({
        "custom:analyst": "none",
        "custom:writer": "core",
        "custom:reviewer": "everything",
        "custom:invalid": "secret",
      }),
  };

  assert.deepEqual(readCatalogPersonaMemoryLevels(storage), {
    "custom:analyst": "none",
    "custom:writer": "core",
    "custom:reviewer": "everything",
  });
  assert.deepEqual(readCatalogPersonaMemoryLevels(null), {});
  assert.deepEqual(readCatalogPersonaMemoryLevels({ getItem: () => "[]" }), {});
});

test("catalog memory levels persist without blocking on storage errors", () => {
  let storedKey = "";
  let storedValue = "";
  writeCatalogPersonaMemoryLevels(
    { "custom:analyst": "core" },
    {
      setItem: (key, value) => {
        storedKey = key;
        storedValue = value;
      },
    },
  );

  assert.equal(storedKey, "buzz-persona-catalog-memory-levels-v1");
  assert.equal(storedValue, '{"custom:analyst":"core"}');
  assert.doesNotThrow(() =>
    writeCatalogPersonaMemoryLevels(
      { "custom:analyst": "core" },
      {
        setItem: () => {
          throw new Error("unavailable");
        },
      },
    ),
  );
});

test("catalog publication versions read only string revisions", () => {
  const storage = {
    getItem: () =>
      JSON.stringify({
        "custom:analyst": "2026-07-22T00:00:00.000Z",
        "custom:invalid": 42,
      }),
  };

  assert.deepEqual(readPublishedCatalogPersonaVersions(storage), {
    "custom:analyst": "2026-07-22T00:00:00.000Z",
  });
  assert.deepEqual(readPublishedCatalogPersonaVersions(null), {});
  assert.deepEqual(
    readPublishedCatalogPersonaVersions({ getItem: () => "[]" }),
    {},
  );
});

test("catalog publication versions persist without blocking on storage errors", () => {
  let storedKey = "";
  let storedValue = "";
  writePublishedCatalogPersonaVersions(
    { "custom:analyst": "2026-07-22T00:00:00.000Z" },
    {
      setItem: (key, value) => {
        storedKey = key;
        storedValue = value;
      },
    },
  );

  assert.equal(storedKey, "buzz-persona-catalog-published-versions-v1");
  assert.equal(storedValue, '{"custom:analyst":"2026-07-22T00:00:00.000Z"}');
  assert.doesNotThrow(() =>
    writePublishedCatalogPersonaVersions(
      { "custom:analyst": "2026-07-22T00:00:00.000Z" },
      {
        setItem: () => {
          throw new Error("unavailable");
        },
      },
    ),
  );
});
