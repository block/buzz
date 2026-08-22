import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

after(() => dom.window.close());

test("Inbox list and auxiliary-pane widths restore from localStorage", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useResizableInboxListWidth } = await import(
    "./useResizableInboxListWidth.ts"
  );
  const { useThreadPanelWidth } = await import(
    "@/shared/hooks/useThreadPanelWidth"
  );

  window.localStorage.clear();
  window.sessionStorage.clear();
  window.localStorage.setItem("buzz.desktop.home-inbox-list-width", "430");
  window.localStorage.setItem("buzz.desktop.thread-panel-width", "460");
  window.sessionStorage.setItem("buzz.desktop.home-inbox-list-width", "310");
  window.sessionStorage.setItem("buzz.desktop.thread-panel-width", "320");

  try {
    const inboxList = renderHook(() => useResizableInboxListWidth());
    const auxiliaryPane = renderHook(() => useThreadPanelWidth(1_200));

    assert.equal(inboxList.result.current.inboxListWidthPx, 430);
    assert.equal(auxiliaryPane.result.current.widthPx, 460);

    act(() => {
      inboxList.result.current.handleInboxListResizeStart({
        clientX: 100,
        preventDefault: () => {},
      });
    });
    const inboxMove = new window.Event("pointermove");
    Object.defineProperty(inboxMove, "clientX", { value: 150 });
    act(() => {
      window.dispatchEvent(inboxMove);
      window.dispatchEvent(new window.Event("pointerup"));
    });
    act(() => {
      auxiliaryPane.result.current.onResizeStart({
        clientX: 500,
        preventDefault: () => {},
      });
    });
    const auxiliaryMove = new window.Event("pointermove");
    Object.defineProperty(auxiliaryMove, "clientX", { value: 450 });
    act(() => {
      window.dispatchEvent(auxiliaryMove);
      window.dispatchEvent(new window.Event("pointerup"));
    });

    assert.equal(
      window.localStorage.getItem("buzz.desktop.home-inbox-list-width"),
      "480",
    );
    assert.equal(
      window.localStorage.getItem("buzz.desktop.thread-panel-width"),
      "510",
    );
    inboxList.unmount();
    auxiliaryPane.unmount();

    const restoredInboxList = renderHook(() => useResizableInboxListWidth());
    const restoredAuxiliaryPane = renderHook(() => useThreadPanelWidth(1_200));
    assert.equal(restoredInboxList.result.current.inboxListWidthPx, 480);
    assert.equal(restoredAuxiliaryPane.result.current.widthPx, 510);
    restoredInboxList.unmount();
    restoredAuxiliaryPane.unmount();
  } finally {
    cleanup();
  }
});
