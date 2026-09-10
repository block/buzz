import { Layout, Model } from "flexlayout-react";
import {
  IconArrowsMaximize,
  IconArrowsMinimize,
  IconChevronDown,
  IconX,
} from "@tabler/icons-react";
import { useState } from "react";
import { Button } from "./Button";
import "../styles/flex-workspace.css";

/** A separate comparison, preserving FlexLayout's own tabs and docking behavior. */
export function FlexWorkspace() {
  const [revision, setRevision] = useState(0);
  return (
    <div className="flex-workspace-comparison">
      <div className="flex-workspace-intro">
        <p className="text-body text-secondary">
          Drag a tab to an edge to split, or into another tab group to stack.
          Resize in the gaps. All four panels—including Navigation—can move
          here; this comparison uses FlexLayout’s own placement rules.
        </p>
        <Button
          size="compact"
          onClick={() => setRevision((value) => value + 1)}
        >
          Reset layout
        </Button>
      </div>
      <div className="flex-workspace-stage">
        <div className="flex-workspace-host">
          <FlexWorkspaceLayout key={revision} />
        </div>
      </div>
      <p className="text-body-sm text-tertiary">
        Empty panels, native tabs, continuous resizing. This layout is local to
        this page and resets when you leave; it does not change the Dockview
        example or app shell.
      </p>
    </div>
  );
}

const icons = {
  close: <IconX size={16} aria-hidden="true" />,
  maximize: <IconArrowsMaximize size={16} aria-hidden="true" />,
  restore: <IconArrowsMinimize size={16} aria-hidden="true" />,
  more: <IconChevronDown size={16} aria-hidden="true" />,
};

function emptyPanel() {
  return null;
}

function FlexWorkspaceLayout() {
  // Keep one model per mounted comparison. Parent rerenders and theme toggles
  // must never discard the arrangement the person is testing.
  const [model] = useState(() =>
    Model.fromJson({
      global: {
        tabEnableClose: false,
        rootOrientationVertical:
          window.matchMedia("(max-width: 48rem)").matches,
      },
      borders: [],
      layout: {
        type: "row",
        children: [
          {
            type: "tabset",
            weight: 25,
            children: [
              {
                type: "tab",
                id: "flex-navigation",
                name: "Navigation",
                component: "empty",
              },
            ],
          },
          {
            type: "tabset",
            weight: 45,
            children: [
              { type: "tab", id: "flex-one", name: "One", component: "empty" },
            ],
          },
          {
            type: "row",
            weight: 30,
            children: [
              {
                type: "tabset",
                children: [
                  {
                    type: "tab",
                    id: "flex-two",
                    name: "Two",
                    component: "empty",
                  },
                ],
              },
              {
                type: "tabset",
                children: [
                  {
                    type: "tab",
                    id: "flex-three",
                    name: "Three",
                    component: "empty",
                  },
                ],
              },
            ],
          },
        ],
      },
    }),
  );
  return (
    <Layout model={model} factory={emptyPanel} icons={icons} realtimeResize />
  );
}
