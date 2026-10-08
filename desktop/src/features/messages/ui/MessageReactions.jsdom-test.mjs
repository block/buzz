import assert from "node:assert/strict";
import { afterEach, mock, test } from "node:test";

import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { DEFAULT_POPOVER_HOVER_OPEN_DELAY_MS } from "@/shared/ui/popover";
import { TooltipProvider } from "@/shared/ui/tooltip";
import {
  MessageReactions,
  REACTION_HOVER_OPEN_DELAY_MS,
} from "./MessageReactions.tsx";

const REACTION = {
  emoji: "👍",
  count: 1,
  users: [{ pubkey: "a".repeat(64), displayName: "bob", avatarUrl: null }],
};

afterEach(() => {
  mock.timers.reset();
});

function mountReactions() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(
      React.createElement(
        TooltipProvider,
        null,
        React.createElement(MessageReactions, {
          messageId: "m".repeat(64),
          reactions: [REACTION],
          canToggle: true,
          pending: false,
          onSelect: () => {},
        }),
      ),
    );
  });
  return {
    pill: [...container.querySelectorAll("button")].find(
      (button) => button.getAttribute("aria-label") === "Toggle 👍 reaction",
    ),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function hover(element) {
  act(() => {
    element.dispatchEvent(
      new MouseEvent("mouseover", { bubbles: true, relatedTarget: null }),
    );
  });
}

function popoverIsOpen() {
  return document.body.textContent?.includes("bob") ?? false;
}

test("hovering a reaction shows who reacted after a short dwell, not the shared popover delay", () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  const { pill, unmount } = mountReactions();
  assert.ok(pill);
  assert.ok(REACTION_HOVER_OPEN_DELAY_MS < DEFAULT_POPOVER_HOVER_OPEN_DELAY_MS);

  hover(pill);
  act(() => mock.timers.tick(REACTION_HOVER_OPEN_DELAY_MS - 1));
  assert.equal(popoverIsOpen(), false, "popover opened before the dwell");

  act(() => mock.timers.tick(1));
  assert.equal(popoverIsOpen(), true, "popover did not open after the dwell");

  unmount();
});
