import { describe, expect, it } from "vitest";

import { COMPONENTS } from "@/shared/ui/registry";

import {
  baseUiBackingSentence,
  flattenSentence,
} from "./baseUiBackingSentence";

function sentenceFor(slug: string): string {
  const component = COMPONENTS.find((candidate) => candidate.slug === slug);
  if (!component) throw new Error(`No component ${slug}`);
  return flattenSentence(
    baseUiBackingSentence(component.slug, component.behavior),
  );
}

describe("the Base UI backing sentence", () => {
  it("names the part a component imports itself", () => {
    expect(sentenceFor("button")).toBe("Built on Base UI Button.");
    expect(sentenceFor("avatar")).toBe("Built on Base UI Avatar.");
    expect(sentenceFor("tabs")).toBe("Built on Base UI Tabs.");
  });

  it("says where an indirect part enters, rather than claiming none", () => {
    expect(sentenceFor("icon-button")).toBe(
      "No Base UI part of its own. Inherits Base UI Button through Buzz Button.",
    );
    expect(sentenceFor("navigator-row")).toBe(
      "No Base UI part of its own. Inherits Base UI Button through Buzz Button.",
    );
  });

  it("reads as one sentence when a component has both", () => {
    expect(sentenceFor("search-field")).toBe(
      "Built on Base UI Field and Input. Also inherits Base UI Button through Buzz Button.",
    );
  });

  it("says what a component is instead when it has no part at all", () => {
    expect(sentenceFor("panel-header")).toBe(
      "No Base UI part. Semantic native header.",
    );
    expect(sentenceFor("inline-chip")).toBe(
      "No Base UI part. Semantic native button or image role.",
    );
  });

  it("gives every component a sentence that ends and never doubles a full stop", () => {
    for (const component of COMPONENTS) {
      const sentence = sentenceFor(component.slug);
      expect(sentence, component.slug).not.toBe("");
      expect(sentence.endsWith("."), component.slug).toBe(true);
      expect(sentence, component.slug).not.toContain("..");
      // A stray double space is the tell that a segment joined without its
      // separator; the sentence is assembled from parts, so check it reads.
      expect(sentence, component.slug).not.toContain("  ");
    }
  });

  it("links every part it names", () => {
    const segments = baseUiBackingSentence("search-field", "unused");
    const linked = segments
      .filter((segment) => segment.kind === "part")
      .map((segment) => (segment.kind === "part" ? segment.part.name : ""));
    expect(linked).toEqual(["Field", "Input", "Button"]);
  });
});
