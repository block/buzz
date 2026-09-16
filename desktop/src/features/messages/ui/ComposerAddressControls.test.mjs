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
const secondAgent = {
  avatarUrl: null,
  displayName: "Agent Bea",
  pubkey: "second-agent-pubkey",
};
const thirdAgent = {
  avatarUrl: null,
  displayName: "Agent Cia",
  pubkey: "third-agent-pubkey",
};

test("mention control expands with automatically mentioned agents", async () => {
  const React = await import("react");
  const { fireEvent, render } = await import("@testing-library/react");
  const { TooltipProvider } = await import("@/shared/ui/tooltip");
  const { ComposerMentionButton } = await import(
    "./ComposerAddressControls.tsx"
  );
  let opened = 0;
  const removed = [];
  const renderButton = (agents) =>
    React.createElement(
      TooltipProvider,
      null,
      React.createElement(ComposerMentionButton, {
        agents,
        disabled: false,
        onCaptureSelection: () => {},
        onOpen: () => {
          opened += 1;
        },
        onRemove: (pubkey) => removed.push(pubkey),
        showAgents: true,
      }),
    );
  const view = render(renderButton([agent]));
  view.rerender(renderButton([agent, secondAgent, thirdAgent]));

  assert.ok(view.getByTestId("composer-address-locks"));
  const avatar = view.getByTestId("composer-address-lock-agent-pubkey");
  assert.ok(avatar);
  const manage = view.getByRole("button", {
    name: "Manage mentions",
  });
  assert.match(manage.className, /(?:^|\s)-ml-2(?:\s|$)/);
  assert.match(manage.className, /(?:^|\s)pl-2(?:\s|$)/);
  assert.match(manage.parentElement?.className ?? "", /(?:^|\s)pl-2(?:\s|$)/);
  assert.match(
    manage.parentElement?.className ?? "",
    /(?:^|\s)pr-1\.5(?:\s|$)/,
  );
  assert.match(
    view.getByRole("button", { name: "Manage mentions" }).parentElement
      ?.className ?? "",
    /(?:^|\s)bg-primary\/15(?:\s|$)/,
  );
  assert.match(
    view.getByRole("button", { name: "Manage mentions" }).parentElement
      ?.className ?? "",
    /(?:^|\s)text-primary(?:\s|$)/,
  );
  assert.doesNotMatch(
    view.getByRole("button", { name: "Manage mentions" }).parentElement
      ?.className ?? "",
    /(?:^|\s)bg-accent\/70(?:\s|$)/,
  );
  assert.doesNotMatch(
    avatar.querySelector("span")?.className ?? "",
    /(?:^|\s)ring(?:-|\s)/,
  );
  for (const addedAgent of [secondAgent, thirdAgent]) {
    const addedAvatar = view.getByTestId(
      `composer-address-lock-${addedAgent.pubkey}`,
    );
    assert.equal(addedAvatar.parentElement?.style.opacity, "0");
    assert.match(
      addedAvatar.parentElement?.style.transform ?? "",
      /scale\(0.8\)/,
    );
  }
  const remove = view.getByRole("button", {
    name: "Don't automatically mention Agent Ada in this thread",
  });
  assert.equal(
    remove.getAttribute("aria-label")?.includes("conversation"),
    false,
  );
  const removeChrome = remove.querySelector("span.absolute");
  assert.match(
    removeChrome?.className ?? "",
    /group-hover\/address:opacity-100/,
  );
  assert.match(removeChrome?.className ?? "", /(?:^|\s)bg-foreground(?:\s|$)/);
  assert.doesNotMatch(
    removeChrome?.className ?? "",
    /(?:^|\s)bg-foreground\/80(?:\s|$)/,
  );
  fireEvent.click(remove);
  assert.deepEqual(removed, ["agent-pubkey"]);
  view.rerender(renderButton([]));
  const exitingLocks = view.getByTestId("composer-address-locks");
  assert.match(exitingLocks.className, /(?:^|\s)overflow-hidden(?:\s|$)/);
  assert.match(
    view.getByRole("button", { name: "Mention someone" }).parentElement
      ?.className ?? "",
    /(?:^|\s)pr-1\.5(?:\s|$)/,
  );
  fireEvent.click(view.getByRole("button", { name: "Mention someone" }));
  assert.equal(opened, 1);
});
