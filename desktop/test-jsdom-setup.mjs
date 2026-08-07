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
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
