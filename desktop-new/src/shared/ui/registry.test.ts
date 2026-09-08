import { describe, expect, it } from "vitest";

import {
  BASE_UI_PARTS,
  COMPONENTS,
  resolveBaseUiBacking,
  baseUiDocsUrl,
} from "./registry";

/**
 * The component sources as text, read through Vite rather than `node:fs` so the
 * paths resolve the same way the app resolves them and no Node types are needed.
 */
const SOURCES = import.meta.glob(
  "/src/{shared/ui,features/composer/ui,features/agent-activity/ui}/*.tsx",
  {
    query: "?raw",
    import: "default",
    eager: true,
  },
) as Record<string, string>;

function read(source: string): string {
  const contents = SOURCES[`/src/${source}`];
  if (contents === undefined) {
    throw new Error(`No source file at src/${source}`);
  }
  return contents;
}

/**
 * The Base UI column on /design/components/base-ui is only worth reading if it
 * reports what the code imports. These tests bind it to the files themselves —
 * a component that gains or loses a Base UI import fails here rather than
 * quietly showing a stale answer on the page.
 */
describe("component registry — Base UI backing", () => {
  it("names a source file that exists for every component", () => {
    for (const component of COMPONENTS) {
      expect(() => read(component.source), component.slug).not.toThrow();
    }
  });

  it("places documented child components beneath a real parent", () => {
    for (const component of COMPONENTS) {
      if (!component.parent) continue;
      const parent = COMPONENTS.find(
        (candidate) => candidate.slug === component.parent,
      );
      expect(parent, `${component.slug} parent`).toBeDefined();
      expect(
        parent?.parent,
        `${component.slug} parent is top-level`,
      ).toBeUndefined();
    }
  });

  it("claims exactly the Base UI modules its source imports", () => {
    for (const component of COMPONENTS) {
      const source = read(component.source);
      const imported = [
        ...source.matchAll(/from "(@base-ui\/react\/[a-z-]+)"/g),
      ].map((match) => match[1]);

      const claimed = component.baseUi.map((part) => part.module);

      expect(new Set(claimed), `${component.slug} claims`).toEqual(
        new Set(imported),
      );
    }
  });

  it("composes only siblings it actually imports", () => {
    for (const component of COMPONENTS) {
      const source = read(component.source);
      for (const slug of component.composes) {
        const sibling = COMPONENTS.find((candidate) => candidate.slug === slug);
        expect(sibling, `${component.slug} composes ${slug}`).toBeDefined();
        expect(
          source.includes(`from "./${sibling?.name}"`) ||
            source.includes(`from "@/shared/ui/${sibling?.name}"`),
          `${component.slug} imports ${sibling?.name}`,
        ).toBe(true);
      }
    }
  });

  it("resolves inherited backing through composition without double-counting", () => {
    // IconButton imports no Base UI part but renders a Buzz Button, so Base UI
    // Button is underneath it — reported as inherited, not own.
    const iconButton = resolveBaseUiBacking("icon-button");
    expect(iconButton.own).toEqual([]);
    expect(iconButton.inherited).toEqual([
      { part: BASE_UI_PARTS.button, through: "Button" },
    ]);

    // SearchField imports Field and Input directly and composes IconButton,
    // which reaches Button — three distinct parts, none listed twice.
    const searchField = resolveBaseUiBacking("search-field");
    expect(searchField.own).toEqual([BASE_UI_PARTS.field, BASE_UI_PARTS.input]);
    expect(searchField.inherited.map((entry) => entry.part.name)).toEqual([
      "Button",
    ]);

    // A component whose own import already covers a part does not also report
    // it as inherited.
    const button = resolveBaseUiBacking("button");
    expect(button.own).toEqual([BASE_UI_PARTS.button]);
    expect(button.inherited).toEqual([]);
  });

  it("builds a docs URL per documented Base UI component slug", () => {
    for (const part of Object.values(BASE_UI_PARTS)) {
      expect(baseUiDocsUrl(part)).toBe(
        `https://base-ui.com/react/components/${part.docs}`,
      );
      expect(part.module).toBe(`@base-ui/react/${part.docs}`);
    }
  });
});
