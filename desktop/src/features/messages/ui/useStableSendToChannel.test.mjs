import assert from "node:assert/strict";
import test from "node:test";

class EventTargetShim {
  addEventListener() {}
  removeEventListener() {}
}
class NodeShim extends EventTargetShim {
  constructor(tagName) {
    super();
    this.nodeType = 1;
    this.nodeName = tagName.toUpperCase();
    this.tagName = tagName;
    this.namespaceURI = "http://www.w3.org/1999/xhtml";
    this.ownerDocument = globalThis.document;
    this.parentNode = null;
    this.children = [];
    this.childNodes = [];
  }
  appendChild(child) {
    this.children.push(child);
    this.childNodes.push(child);
    child.parentNode = this;
    return child;
  }
  removeChild(child) {
    this.children = this.children.filter((current) => current !== child);
    this.childNodes = this.childNodes.filter((current) => current !== child);
    child.parentNode = null;
    return child;
  }
}
class DocumentShim extends EventTargetShim {
  constructor() {
    super();
    this.nodeType = 9;
    this.defaultView = globalThis;
  }
  createElement(tagName) {
    return new NodeShim(tagName);
  }
}
globalThis.document = new DocumentShim();
globalThis.HTMLIFrameElement = NodeShim;
globalThis.HTMLDivElement = NodeShim;
globalThis.HTMLElement = NodeShim;
globalThis.Node = NodeShim;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
Object.defineProperty(globalThis, "window", {
  configurable: true,
  value: globalThis,
});

import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { useStableSendToChannel } from "./useStableSendToChannel.ts";

function Harness({ channelId, onSendToChannel, threadHead, output }) {
  output.current = useStableSendToChannel(
    channelId,
    threadHead,
    onSendToChannel,
  );
  return null;
}

test("send-to-channel callback stays stable while using the latest thread context", async () => {
  const output = { current: undefined };
  const calls = [];
  const firstRoot = { id: "first" };
  const secondRoot = { id: "second" };
  const firstSend = async (...args) => calls.push(["first", ...args]);
  const secondSend = async (...args) => calls.push(["second", ...args]);
  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(
      React.createElement(Harness, {
        channelId: "channel-a",
        onSendToChannel: firstSend,
        threadHead: firstRoot,
        output,
      }),
    );
  });
  const initialCallback = output.current;

  await act(async () => {
    root.render(
      React.createElement(Harness, {
        channelId: "channel-b",
        onSendToChannel: secondSend,
        threadHead: secondRoot,
        output,
      }),
    );
  });

  assert.equal(output.current, initialCallback);
  const message = { id: "reply" };
  await output.current(message);
  assert.deepEqual(calls, [["second", message, secondRoot, "channel-b"]]);

  await act(async () => root.unmount());
});
