import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    Element: dom.window.Element,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    Node: dom.window.Node,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

const agent = {
  avatarUrl: null,
  displayName: "Agent Ada",
  pubkey: "agent-pubkey",
};

test("mention-button prototype expands the mention control with addressed agents", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { TooltipProvider } = await import("@/shared/ui/tooltip");
  const { ComposerMentionButton } = await import(
    "./ComposerAddressControls.tsx"
  );
  let opened = 0;
  const view = render(
    React.createElement(
      TooltipProvider,
      null,
      React.createElement(ComposerMentionButton, {
        agents: [agent],
        disabled: false,
        onCaptureSelection: () => {},
        onOpen: () => {
          opened += 1;
        },
        showAgents: true,
      }),
    ),
  );

  assert.ok(view.getByTestId("composer-address-locks"));
  assert.ok(view.getByTestId("composer-address-lock-agent-pubkey"));
  assert.equal(
    view.queryByRole("button", { name: "Stop always mentioning Agent Ada" }),
    null,
  );
  fireEvent.click(
    view.getByRole("button", { name: "Manage mentioned agents" }),
  );
  assert.equal(opened, 1);
});

test("send-button prototype exposes each addressed avatar as a remove control", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { TooltipProvider } = await import("@/shared/ui/tooltip");
  const { ComposerSendButton } = await import("./ComposerAddressControls.tsx");
  const removed = [];
  const view = render(
    React.createElement(
      TooltipProvider,
      null,
      React.createElement(ComposerSendButton, {
        agents: [agent],
        isSending: false,
        onRemove: (pubkey) => removed.push(pubkey),
        sendDisabled: true,
        showAgents: true,
      }),
    ),
  );

  assert.ok(view.getByTestId("composer-address-send"));
  const remove = view.getByRole("button", {
    name: "Stop always mentioning Agent Ada",
  });
  assert.match(
    remove.querySelector("span.absolute")?.className ?? "",
    /group-hover\/address:opacity-100/,
  );
  fireEvent.click(remove);
  assert.deepEqual(removed, ["agent-pubkey"]);
});
