import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

import { installLocalStorage } from "../../playground/lib/testStorage.mjs";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    MutationObserver: dom.window.MutationObserver,
    ResizeObserver: class {
      observe() {}
      disconnect() {}
    },
    window: dom.window,
  });
  installLocalStorage(dom.window.localStorage);
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  const { resetPopoutWindowsForTests } = await import(
    "../lib/popoutWindows.ts"
  );
  resetPopoutWindowsForTests();
  const { resetPopoutSettingsForTests } = await import(
    "../lib/popoutSettings.ts"
  );
  const { resetEmbeddedWindowsForTests } = await import(
    "../lib/embeddedWindows.ts"
  );
  resetPopoutSettingsForTests();
  resetEmbeddedWindowsForTests();
  globalThis.localStorage?.clear();
  delete globalThis.__TAURI_INTERNALS__;
  delete dom.window.__TAURI_INTERNALS__;
  delete globalThis.isTauri;
  delete dom.window.isTauri;
});

after(() => dom.window.close());

async function renderSection() {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { SidebarProvider } = await import("@/shared/ui/sidebar.tsx");
  const { TooltipProvider } = await import("@/shared/ui/tooltip.tsx");
  const { WindowsSection } = await import("./WindowsSection.tsx");
  render(
    createElement(
      TooltipProvider,
      null,
      createElement(SidebarProvider, null, createElement(WindowsSection)),
    ),
  );
  return screen;
}

function installTauriInvoke() {
  const invokes = [];
  const internals = {
    invoke(cmd, args) {
      invokes.push({ cmd, args });
      return Promise.resolve();
    },
  };
  globalThis.isTauri = true;
  dom.window.isTauri = true;
  globalThis.__TAURI_INTERNALS__ = internals;
  dom.window.__TAURI_INTERNALS__ = internals;
  return invokes;
}

test("Windows section is hidden when empty and visible with a pop-out", async () => {
  const screen = await renderSection();
  assert.equal(screen.queryByTestId("windows-section"), null);

  const { act } = await import("@testing-library/react");
  const { setPopoutWindowsForTests } = await import("../lib/popoutWindows.ts");
  await act(() => {
    setPopoutWindowsForTests([
      { label: "main", title: "Buzz" },
      { label: "popout-thread-aaa", title: "Thread" },
    ]);
  });
  assert.ok(screen.getByTestId("windows-section"));
  assert.equal(
    screen.getByTestId("windows-section-label").textContent,
    "Windows",
  );
  assert.ok(screen.getByTestId("open-window-popout-thread-aaa"));
  assert.equal(screen.queryByTestId("open-window-main"), null);
});

test("Clearing the last pop-out hides the Windows section", async () => {
  const { act } = await import("@testing-library/react");
  const { setPopoutWindowsForTests } = await import("../lib/popoutWindows.ts");
  setPopoutWindowsForTests([{ label: "popout-playground-bbb", title: "Demo" }]);
  const screen = await renderSection();
  assert.ok(screen.getByTestId("windows-section"));
  await act(() => {
    setPopoutWindowsForTests([]);
  });
  assert.equal(screen.queryByTestId("windows-section"), null);
});

test("Show Windows off hides the section even when pop-outs exist", async () => {
  const { setPopoutWindowsForTests } = await import("../lib/popoutWindows.ts");
  const { setShowWindowsSection } = await import("../lib/popoutSettings.ts");
  setPopoutWindowsForTests([{ label: "popout-thread-aaa", title: "Thread" }]);
  setShowWindowsSection(false);
  const screen = await renderSection();
  assert.equal(screen.queryByTestId("windows-section"), null);
});

test("embed mode lists embedded windows and can close them", async () => {
  const { act } = await import("@testing-library/react");
  const { setEmbedInMain, setShowWindowsSection } = await import(
    "../lib/popoutSettings.ts"
  );
  const { openEmbeddedWindow, closeEmbeddedWindow } = await import(
    "../lib/embeddedWindows.ts"
  );
  setShowWindowsSection(true);
  setEmbedInMain(true);
  openEmbeddedWindow({
    label: "popout-thread-aaa",
    payload: { kind: "thread", title: "Design review", threadId: "t1" },
  });
  const screen = await renderSection();
  assert.ok(screen.getByTestId("windows-section"));
  assert.ok(screen.getByTestId("open-window-popout-thread-aaa"));
  assert.ok(screen.getByTestId("close-window-popout-thread-aaa"));
  await act(() => {
    closeEmbeddedWindow("popout-thread-aaa");
  });
  assert.equal(screen.queryByTestId("windows-section"), null);
});

test("clicking an OS window row focuses the native pop-out, not embed", async () => {
  const { fireEvent, act } = await import("@testing-library/react");
  const { setPopoutWindowsForTests } = await import("../lib/popoutWindows.ts");
  const { getActiveEmbeddedWindow, openEmbeddedWindow } = await import(
    "../lib/embeddedWindows.ts"
  );
  const { setShowWindowsSection, setEmbedInMain } = await import(
    "../lib/popoutSettings.ts"
  );
  setShowWindowsSection(true);
  setEmbedInMain(false);
  openEmbeddedWindow({
    label: "popout-thread-aaa",
    payload: { kind: "thread", title: "Design review", threadId: "t1" },
  });
  setPopoutWindowsForTests([{ label: "popout-thread-aaa", title: "Thread" }]);
  const screen = await renderSection();
  const invokes = installTauriInvoke();

  await act(async () => {
    fireEvent.click(screen.getByTestId("open-window-popout-thread-aaa"));
  });

  const focusCalls = invokes.filter(
    (call) => call.cmd === "focus_popout_window",
  );
  assert.equal(focusCalls.length, 1);
  assert.deepEqual(focusCalls[0].args, { label: "popout-thread-aaa" });
  assert.equal(
    invokes.some((call) => call.cmd === "open_popout_window"),
    false,
  );
  assert.equal(getActiveEmbeddedWindow(), null);
});

test("clicking an embedded window row activates embed, not native focus", async () => {
  const { fireEvent, act } = await import("@testing-library/react");
  const { setEmbedInMain, setShowWindowsSection } = await import(
    "../lib/popoutSettings.ts"
  );
  const { openEmbeddedWindow, getActiveEmbeddedWindow } = await import(
    "../lib/embeddedWindows.ts"
  );
  setShowWindowsSection(true);
  setEmbedInMain(true);
  openEmbeddedWindow({
    label: "popout-thread-aaa",
    payload: { kind: "thread", title: "Design review", threadId: "t1" },
  });
  openEmbeddedWindow({
    label: "popout-playground-bbb",
    payload: { kind: "playground", title: "Demo" },
  });
  const screen = await renderSection();
  const invokes = installTauriInvoke();
  assert.equal(getActiveEmbeddedWindow()?.label, "popout-playground-bbb");

  await act(async () => {
    fireEvent.click(screen.getByTestId("open-window-popout-thread-aaa"));
  });

  assert.equal(getActiveEmbeddedWindow()?.label, "popout-thread-aaa");
  assert.equal(
    invokes.some((call) => call.cmd === "focus_popout_window"),
    false,
  );
});
