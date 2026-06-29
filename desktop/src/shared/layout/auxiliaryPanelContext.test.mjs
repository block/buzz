import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { AuxiliaryPanel } from "./AuxiliaryPanel/index.ts";
import { AuxiliaryPanelBody } from "./AuxiliaryPanel/index.ts";
import {
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
} from "./AuxiliaryPanel/index.ts";
import {
  AuxiliaryPanelContext,
  useAuxiliaryPanel,
} from "./AuxiliaryPanel/index.ts";

function render(element) {
  return renderToStaticMarkup(element);
}

test("AuxiliaryPanel provides layout mode through context", () => {
  function ContextProbe() {
    const context = useAuxiliaryPanel();
    return React.createElement(
      "span",
      null,
      `${context.mode}:${context.layout}:${context.isSplitLayout}`,
    );
  }

  const html = render(
    React.createElement(
      AuxiliaryPanel,
      {
        layout: "split",
        onClose: () => {},
        widthPx: 420,
      },
      React.createElement(ContextProbe),
    ),
  );

  assert.match(html, /docked:split:true/);
});

test("AuxiliaryPanelBody accepts a mode override and applies panel padding", () => {
  const html = render(
    React.createElement(
      AuxiliaryPanelBody,
      {
        className: "overflow-y-auto",
        mode: "panel",
        panelPadding: true,
      },
      "Panel body",
    ),
  );

  assert.match(html, /min-h-0/);
  assert.match(html, /flex-1/);
  assert.match(html, /pt-4/);
  assert.match(html, /overflow-y-auto/);
  assert.match(html, />Panel body</);
});

test("AuxiliaryPanelBody resolves mode from context", () => {
  const html = render(
    React.createElement(
      AuxiliaryPanelContext.Provider,
      {
        value: {
          isFloatingOverlay: false,
          isOverlay: false,
          isSinglePanelView: false,
          isSplitLayout: false,
          layout: "standalone",
          mode: "single-panel",
          onClose: () => {},
          transparentChrome: false,
          widthPx: 360,
        },
      },
      React.createElement(AuxiliaryPanelBody, null, "Body"),
    ),
  );

  assert.match(html, /pt-13/);
});

test("AuxiliaryPanelBody throws without a mode or provider", () => {
  assert.throws(
    () => render(React.createElement(AuxiliaryPanelBody, null, "Body")),
    /AuxiliaryPanelBody requires `mode` or an AuxiliaryPanel ancestor/,
  );
});

test("useAuxiliaryPanel throws outside AuxiliaryPanel", () => {
  function HookProbe() {
    useAuxiliaryPanel();
    return React.createElement("span", null, "unreachable");
  }

  assert.throws(
    () => render(React.createElement(HookProbe)),
    /useAuxiliaryPanel must be used within AuxiliaryPanel/,
  );
});

test("AuxiliaryPanelHeaderGroup derives overlay button styling from context", () => {
  const html = render(
    React.createElement(
      AuxiliaryPanelContext.Provider,
      {
        value: {
          isFloatingOverlay: true,
          isOverlay: true,
          isSinglePanelView: false,
          isSplitLayout: false,
          layout: "standalone",
          mode: "panel",
          onClose: () => {},
          transparentChrome: false,
          widthPx: 360,
        },
      },
      React.createElement(
        AuxiliaryPanelHeader,
        null,
        React.createElement(
          AuxiliaryPanelHeaderGroup,
          { onBack: () => {} },
          "Title",
        ),
      ),
    ),
  );

  assert.match(html, /ml-0/);
  assert.doesNotMatch(html, /-ml-2/);
});

test("AuxiliaryPanel applies className in standalone layout", () => {
  const html = render(
    React.createElement(
      AuxiliaryPanel,
      {
        className: "custom-panel-class",
        onClose: () => {},
        widthPx: 420,
      },
      "Panel",
    ),
  );

  assert.match(html, /custom-panel-class/);
});

test("AuxiliaryPanel resize handle uses a generic namespace", () => {
  const html = render(
    React.createElement(
      AuxiliaryPanel,
      {
        onClose: () => {},
        onResizeStart: () => {},
        widthPx: 420,
      },
      "Panel",
    ),
  );

  assert.match(html, /peer\/auxiliary-panel-resize/);
  assert.match(html, /group\/auxiliary-panel-resize/);
  assert.doesNotMatch(html, /profile-resize/);
});
