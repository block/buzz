// Shared helpers for sidebar sync manager tests.

export function makeFakeWindow() {
  const storage = new Map();
  const ls = {
    getItem: (k) => storage.get(k) ?? null,
    setItem: (k, v) => storage.set(k, v),
    removeItem: (k) => storage.delete(k),
    clear: () => storage.clear(),
  };
  let timerCallback = null;
  let nextTimerId = 100;
  return {
    localStorage: ls,
    setTimeout: (fn, _ms) => {
      timerCallback = fn;
      return nextTimerId++;
    },
    clearTimeout: (_id) => {
      timerCallback = null;
    },
    _fireTimer: () => {
      if (timerCallback) {
        const fn = timerCallback;
        timerCallback = null;
        fn();
      }
    },
    _hasTimer: () => timerCallback !== null,
  };
}

export function installFakeWindow(fw) {
  if (typeof globalThis.window === "undefined") globalThis.window = {};
  const origLs = globalThis.window.localStorage;
  const origSt = globalThis.window.setTimeout;
  const origCt = globalThis.window.clearTimeout;
  globalThis.window.localStorage = fw.localStorage;
  globalThis.window.setTimeout = fw.setTimeout;
  globalThis.window.clearTimeout = fw.clearTimeout;
  return () => {
    if (origLs !== undefined) globalThis.window.localStorage = origLs;
    if (origSt !== undefined) globalThis.window.setTimeout = origSt;
    if (origCt !== undefined) globalThis.window.clearTimeout = origCt;
  };
}

export function installTauriMock(goodCipherPayload) {
  const orig = globalThis.window?.__TAURI_INTERNALS__;
  if (typeof globalThis.window === "undefined") globalThis.window = {};
  let captured = null;
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      if (cmd === "nip44_decrypt_from_self") {
        if (args?.ciphertext === "bad-cipher")
          return Promise.reject(new Error("decrypt failed"));
        return Promise.resolve(goodCipherPayload);
      }
      if (cmd === "nip44_encrypt_to_self") {
        captured = args?.plaintext ?? null;
        return Promise.resolve("ct");
      }
      if (cmd === "sign_event")
        return Promise.resolve(
          JSON.stringify({
            id: "eid",
            pubkey: "pk-lww",
            content: "ct",
            created_at: args?.createdAt ?? 0,
            kind: args?.kind ?? 0,
            tags: args?.tags ?? [],
            sig: "s",
          }),
        );
      return Promise.reject(new Error(`unmocked: ${cmd}`));
    },
  };
  return {
    restore: () => {
      if (orig !== undefined) globalThis.window.__TAURI_INTERNALS__ = orig;
      else delete globalThis.window.__TAURI_INTERNALS__;
    },
    capturedPlaintext: () => captured,
  };
}
