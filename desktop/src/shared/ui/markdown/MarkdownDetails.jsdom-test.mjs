import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { fireEvent } from "@testing-library/react";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

import {
  clearOpenMarkdownSections,
  MarkdownDetails,
  MarkdownDetailsSummary,
} from "./MarkdownDetails.tsx";
import { MarkdownRuntimeContext } from "./runtimeContext.ts";

const roots = [];

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
  clearOpenMarkdownSections();
});

function section({ messageId = "event-1", detailsKey, title, interactive }) {
  return React.createElement(
    MarkdownRuntimeContext.Provider,
    {
      value: {
        channels: [],
        messageId,
        onOpenChannel: () => {},
        onOpenEntityLink: () => {},
        onOpenMessageLink: () => {},
        relayOrigin: null,
      },
    },
    React.createElement(
      MarkdownDetails,
      { detailsKey, interactive },
      React.createElement(MarkdownDetailsSummary, { key: "s" }, title),
      React.createElement("p", { key: "b" }, `${title} body`),
    ),
  );
}

function mount(props) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(section(props)));
  roots.push({ root, container });
  return {
    container,
    rerender: (next) => act(() => root.render(section(next))),
    toggle: () => act(() => fireEvent.click(container.querySelector("button"))),
    expanded: () =>
      container.querySelector("button")?.getAttribute("aria-expanded"),
  };
}

test("MarkdownDetails: starts collapsed and toggles its body", () => {
  const view = mount({ detailsKey: "0:A", title: "A", interactive: true });
  assert.equal(view.expanded(), "false");
  assert.doesNotMatch(view.container.textContent, /A body/);
  view.toggle();
  assert.equal(view.expanded(), "true");
  assert.match(view.container.textContent, /A body/);
});

test("MarkdownDetails: an open section survives a remount", () => {
  const first = mount({ detailsKey: "0:A", title: "A", interactive: true });
  first.toggle();
  const second = mount({ detailsKey: "0:A", title: "A", interactive: true });
  assert.equal(second.expanded(), "true");
});

test("MarkdownDetails: a renamed section does not inherit the open state", () => {
  const view = mount({ detailsKey: "0:A", title: "A", interactive: true });
  view.toggle();
  view.rerender({ detailsKey: "0:B", title: "B", interactive: true });
  assert.equal(view.expanded(), "false");
  view.rerender({ detailsKey: "0:A", title: "A", interactive: true });
  assert.equal(view.expanded(), "true");
});

test("MarkdownDetails: read-only surfaces show title and body inline", () => {
  const view = mount({ detailsKey: "0:A", title: "A", interactive: false });
  assert.equal(view.container.querySelector("button"), null);
  assert.match(view.container.textContent, /A body/);
});

test("MarkdownDetails: a heading title keeps the toggle inside the heading", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ root, container });
  act(() =>
    root.render(
      React.createElement(
        MarkdownDetails,
        { detailsKey: "0:H", interactive: true },
        React.createElement(
          "h2",
          { key: "h" },
          React.createElement(
            MarkdownDetailsSummary,
            { "data-heading": "" },
            "H",
          ),
        ),
        React.createElement("p", { key: "b" }, "H body"),
      ),
    ),
  );
  const button = container.querySelector("h2 > button");
  assert.ok(button);
  act(() => fireEvent.click(button));
  assert.equal(button.getAttribute("aria-expanded"), "true");
  assert.match(container.textContent, /H body/);
});
