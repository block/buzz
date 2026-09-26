/**
 * Mounted-render tests for the two archive-retention sections added in P3':
 * ObserverRetentionSection and ArchiveSizeSection.
 *
 * These mount the REAL subcomponents (exported from LocalArchiveSettingsCard)
 * against a mocked Tauri IPC bridge, so the render → validate → invoke path is
 * exercised end-to-end. The full card is not mounted: it depends on the
 * communities/channels query context, which is orthogonal to what these
 * sections do. The bound-parsing and byte-formatting logic they build on is
 * unit-tested directly in `retentionSettings.test.mjs`.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const ipcHandlers = new Map();

let act;
let cleanup;
let fireEvent;
let render;
let screen;
let createElement;
let ObserverRetentionSection;
let ArchiveSizeSection;
let LocalArchiveSettingsCard;
let QueryClient;
let QueryClientProvider;
let CommunitiesProvider;

before(async () => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    window: dom.window,
    localStorage: dom.window.localStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
    writable: true,
  });
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      const handler = ipcHandlers.get(cmd);
      if (handler) return handler(args);
      return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
    },
    transformCallback: () => Math.random(),
  };

  ({ act, cleanup, fireEvent, render, screen } = await import(
    "@testing-library/react"
  ));
  ({ createElement } = await import("react"));
  ({ ObserverRetentionSection, ArchiveSizeSection, LocalArchiveSettingsCard } =
    await import("./LocalArchiveSettingsCard.tsx"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
});

afterEach(() => {
  cleanup?.();
  ipcHandlers.clear();
});

after(() => dom.window.close());

// ── ObserverRetentionSection ──────────────────────────────────────────────────

test("retention field seeds from the saved value and disables Save while unchanged", async () => {
  await act(async () => {
    render(
      createElement(ObserverRetentionSection, {
        savedDays: 30,
        status: "loaded",
        onRetry: () => {},
        onSaved: () => {},
      }),
    );
  });

  const input = screen.getByTestId("local-archive-retention-days");
  assert.equal(input.value, "30");
  const save = screen.getByTestId("local-archive-retention-save");
  assert.equal(save.disabled, true, "unchanged value cannot be saved");
});

test("input disabled until the saved value loads", async () => {
  await act(async () => {
    render(
      createElement(ObserverRetentionSection, {
        savedDays: null,
        status: "loading",
        onRetry: () => {},
        onSaved: () => {},
      }),
    );
  });

  assert.equal(
    screen.getByTestId("local-archive-retention-days").disabled,
    true,
  );
});

test("out-of-bounds input surfaces an inline error and blocks Save", async () => {
  await act(async () => {
    render(
      createElement(ObserverRetentionSection, {
        savedDays: 30,
        status: "loaded",
        onRetry: () => {},
        onSaved: () => {},
      }),
    );
  });

  const input = screen.getByTestId("local-archive-retention-days");
  await act(async () => {
    fireEvent.change(input, { target: { value: "0" } });
  });

  assert.match(
    screen.getByTestId("local-archive-retention-error").textContent,
    /at least 1/,
  );
  assert.equal(
    screen.getByTestId("local-archive-retention-save").disabled,
    true,
  );
});

test("inline validation error is associated with the input via aria", async () => {
  await act(async () => {
    render(
      createElement(ObserverRetentionSection, {
        savedDays: 30,
        status: "loaded",
        onRetry: () => {},
        onSaved: () => {},
      }),
    );
  });

  const input = screen.getByTestId("local-archive-retention-days");

  // Valid state: no error wiring.
  assert.equal(input.getAttribute("aria-invalid"), "false");
  assert.equal(input.getAttribute("aria-describedby"), null);

  await act(async () => {
    fireEvent.change(input, { target: { value: "0" } });
  });

  const error = screen.getByTestId("local-archive-retention-error");
  assert.equal(input.getAttribute("aria-invalid"), "true");
  assert.equal(input.getAttribute("aria-describedby"), error.id);
  assert.ok(error.id, "error message carries an id to reference");
});

test("retention load failure shows an error state with retry, not a stuck field", async () => {
  let retries = 0;
  await act(async () => {
    render(
      createElement(ObserverRetentionSection, {
        savedDays: null,
        status: "error",
        onRetry: () => {
          retries += 1;
        },
        onSaved: () => {},
      }),
    );
  });

  // The input/Save affordance is replaced by an explicit error+retry row.
  assert.equal(screen.queryByTestId("local-archive-retention-days"), null);
  const errorState = screen.getByTestId("local-archive-retention-error-state");
  assert.match(errorState.textContent, /Couldn't load/);

  await act(async () => {
    fireEvent.click(errorState.querySelector("button"));
  });
  assert.equal(retries, 1, "Retry invokes the reload callback");
});

test("a valid change saves through the invoke and reports the new value", async () => {
  const setCalls = [];
  ipcHandlers.set("set_observer_retention_days", (args) => {
    setCalls.push(args);
    return Promise.resolve(null);
  });

  let saved = null;
  await act(async () => {
    render(
      createElement(ObserverRetentionSection, {
        savedDays: 30,
        status: "loaded",
        onRetry: () => {},
        onSaved: (d) => {
          saved = d;
        },
      }),
    );
  });

  const input = screen.getByTestId("local-archive-retention-days");
  await act(async () => {
    fireEvent.change(input, { target: { value: "14" } });
  });

  const save = screen.getByTestId("local-archive-retention-save");
  assert.equal(save.disabled, false, "a changed valid value enables Save");

  await act(async () => {
    fireEvent.click(save);
  });

  assert.deepEqual(setCalls, [{ days: 14 }], "invoke receives the parsed days");
  assert.equal(saved, 14, "onSaved reports the persisted value");
});

// ── ArchiveSizeSection ────────────────────────────────────────────────────────

test("size readout shows physical size and reclaimable bytes", async () => {
  await act(async () => {
    render(
      createElement(ArchiveSizeSection, {
        status: "loaded",
        onRetry: () => {},
        stats: {
          mainFileBytes: 1024 ** 3,
          walFileBytes: 512 * 1024 ** 2,
          pageSize: 4096,
          pageCount: 300_000,
          freelistCount: 262_144, // 1 GB reclaimable
        },
      }),
    );
  });

  assert.equal(
    screen.getByTestId("local-archive-size-physical").textContent,
    "1.5 GB",
  );
  const copy = screen.getByTestId("local-archive-size-section").textContent;
  assert.match(copy, /1\.5 GB/);
  assert.match(copy, /1\.0 GB is reclaimable/);
  assert.match(copy, /only shrinks after an offline compaction/);
});

test("size readout shows a loading fallback before stats arrive", async () => {
  await act(async () => {
    render(
      createElement(ArchiveSizeSection, {
        stats: null,
        status: "loading",
        onRetry: () => {},
      }),
    );
  });

  assert.equal(
    screen.getByTestId("local-archive-size-physical").textContent,
    "—",
  );
  assert.match(
    screen.getByTestId("local-archive-size-section").textContent,
    /Loading…/,
  );
});

test("size load failure shows an error state with retry", async () => {
  let retries = 0;
  await act(async () => {
    render(
      createElement(ArchiveSizeSection, {
        stats: null,
        status: "error",
        onRetry: () => {
          retries += 1;
        },
      }),
    );
  });

  assert.equal(screen.queryByTestId("local-archive-size-physical"), null);
  const errorState = screen.getByTestId("local-archive-size-error-state");
  assert.match(errorState.textContent, /Couldn't load/);

  await act(async () => {
    fireEvent.click(errorState.querySelector("button"));
  });
  assert.equal(retries, 1, "Retry invokes the reload callback");
});

// ── Full-card get → set → get round trip ──────────────────────────────────────

test("full card loads the persisted retention, saves an edit, and reflects it", async () => {
  // No stored communities ⇒ activeCommunity is null ⇒ the channels query stays
  // disabled, so only these archive commands fire. get_observer_retention_days
  // reads the store on mount; set persists; the card advances its own state
  // from the resolved set (no re-fetch), which is the get→set→get contract.
  let stored = 30;
  const setCalls = [];
  ipcHandlers.set("get_identity", () =>
    Promise.resolve({ pubkey: "f".repeat(64), display_name: "Tester" }),
  );
  ipcHandlers.set("list_save_subscriptions", () => Promise.resolve([]));
  ipcHandlers.set("get_observer_retention_days", () => Promise.resolve(stored));
  ipcHandlers.set("archive_size_stats", () =>
    Promise.resolve({
      mainFileBytes: 1024 ** 3,
      walFileBytes: 0,
      pageSize: 4096,
      pageCount: 262_144,
      freelistCount: 0,
    }),
  );
  ipcHandlers.set("set_observer_retention_days", (args) => {
    setCalls.push(args);
    stored = args.days;
    return Promise.resolve(null);
  });

  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });

  await act(async () => {
    render(
      createElement(
        QueryClientProvider,
        { client },
        createElement(
          CommunitiesProvider,
          null,
          createElement(LocalArchiveSettingsCard),
        ),
      ),
    );
  });

  // get: field seeded from the persisted 30, Save disabled while unchanged.
  const input = screen.getByTestId("local-archive-retention-days");
  assert.equal(input.value, "30");
  assert.equal(
    screen.getByTestId("local-archive-retention-save").disabled,
    true,
  );

  // set: edit to a new valid value and save.
  await act(async () => {
    fireEvent.change(input, { target: { value: "7" } });
  });
  await act(async () => {
    fireEvent.click(screen.getByTestId("local-archive-retention-save"));
  });

  assert.deepEqual(setCalls, [{ days: 7 }], "set invoke receives parsed days");
  assert.equal(stored, 7, "backing store now holds the new value");

  // get (post-set): the card advanced its saved value, so the field still shows
  // 7 and Save has returned to disabled — the persisted value round-trips.
  assert.equal(input.value, "7");
  assert.equal(
    screen.getByTestId("local-archive-retention-save").disabled,
    true,
    "Save re-disables once the edit matches the new saved value",
  );

  client.clear();
});
