import assert from "node:assert/strict";
import test from "node:test";

import {
  __resetSafeStorageWarningsForTests,
  getStorageItem,
  removeStorageItem,
  setStorageItem,
} from "./safeStorage.ts";

function createThrowingStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    values,
    getItem: (key) => {
      if (key === "__THROW__") throw new Error("SecurityError");
      return values.get(key) ?? null;
    },
    setItem: (key, value) => {
      if (key === "__THROW__") throw new Error("SecurityError");
      values.set(key, String(value));
    },
    removeItem: (key) => {
      if (key === "__THROW__") throw new Error("SecurityError");
      values.delete(key);
    },
  };
}

function patchLocalStorage(storage) {
  globalThis.window ??= {};
  const original = globalThis.window.localStorage;
  Object.defineProperty(globalThis.window, "localStorage", {
    configurable: true,
    writable: true,
    value: storage,
  });
  return () => {
    if (original === undefined) {
      delete globalThis.window.localStorage;
      return;
    }
    Object.defineProperty(globalThis.window, "localStorage", {
      configurable: true,
      writable: true,
      value: original,
    });
  };
}

test.beforeEach(() => {
  __resetSafeStorageWarningsForTests();
});

test("getStorageItem returns the stored value when storage is healthy", () => {
  const restore = patchLocalStorage(
    createThrowingStorage({ "buzz-theme": "buzz" }),
  );
  try {
    assert.equal(getStorageItem("buzz-theme"), "buzz");
    assert.equal(getStorageItem("missing"), null);
  } finally {
    restore();
  }
});

test("getStorageItem returns null when the key is absent", () => {
  const restore = patchLocalStorage(createThrowingStorage());
  try {
    assert.equal(getStorageItem("buzz-theme"), null);
  } finally {
    restore();
  }
});

test("getStorageItem swallows SecurityError and returns the fallback", () => {
  const restore = patchLocalStorage(createThrowingStorage());
  try {
    assert.equal(getStorageItem("__THROW__"), null);
    assert.equal(getStorageItem("__THROW__", "fallback"), "fallback");
  } finally {
    restore();
  }
});

test("setStorageItem returns true on a healthy write", () => {
  const storage = createThrowingStorage();
  const restore = patchLocalStorage(storage);
  try {
    assert.equal(setStorageItem("buzz-theme", "buzz-dark"), true);
    assert.equal(storage.values.get("buzz-theme"), "buzz-dark");
  } finally {
    restore();
  }
});

test("setStorageItem returns false instead of throwing on denied storage", () => {
  const restore = patchLocalStorage(createThrowingStorage());
  try {
    assert.equal(setStorageItem("__THROW__", "x"), false);
  } finally {
    restore();
  }
});

test("removeStorageItem returns false instead of throwing on denied storage", () => {
  const restore = patchLocalStorage(createThrowingStorage());
  try {
    assert.equal(removeStorageItem("__THROW__"), false);
  } finally {
    restore();
  }
});
