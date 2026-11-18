// Install jsdom globals before any test module (including React) is evaluated.
// This ensures React's canUseDOM = true so isInputEventSupported is set correctly.
import { JSDOM } from "jsdom";
const dom = new JSDOM("<!DOCTYPE html>", { url: "http://localhost" });
const jsdomWindow = dom.window;
globalThis.window = jsdomWindow;
globalThis.document = jsdomWindow.document;
for (const key of Object.getOwnPropertyNames(jsdomWindow)) {
  if (!(key in globalThis)) {
    try {
      globalThis[key] = jsdomWindow[key];
    } catch {}
  }
}
// Override Node 24's built-in Event/CustomEvent with jsdom's implementations.
// Node 24 already defines these globals, so the for..in copy-when-absent loop
// above does not replace them. Radix UI constructs events from globalThis
// constructors; when those are the Node built-ins, jsdom 27 correctly rejects
// the resulting instances in dispatchEvent (type-check mismatch). Assigning
// the jsdom versions here ensures Radix's CustomEvent instances are recognised
// as valid by jsdom's dispatchEvent.
globalThis.Event = jsdomWindow.Event;
globalThis.CustomEvent = jsdomWindow.CustomEvent;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
