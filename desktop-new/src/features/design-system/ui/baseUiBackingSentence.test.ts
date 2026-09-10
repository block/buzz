import { describe, expect, it } from "vitest";
import { COMPONENTS } from "@/shared/ui/registry";
import {
  baseUiBackingSentence,
  flattenSentence,
} from "./baseUiBackingSentence";

const sentenceFor = (slug: string) =>
  flattenSentence(baseUiBackingSentence(slug));

describe("inline inheritance description", () => {
  it("names direct Base UI imports", () => {
    expect(sentenceFor("button")).toBe("Inherits Base UI Button.");
    expect(sentenceFor("avatar")).toBe("Inherits Base UI Avatar.");
    expect(sentenceFor("tabs")).toBe("Inherits Base UI Tabs.");
  });
  it("names Buzz owners for indirect backing without duplicating them", () => {
    expect(sentenceFor("composer")).toBe("Inherits Button.");
    expect(sentenceFor("icon-button")).toBe("Inherits Button.");
    expect(sentenceFor("inline-chip")).toBe("Inherits PreviewCard.");
  });
  it("combines both libraries in one sentence", () => {
    expect(sentenceFor("search-field")).toBe(
      "Inherits Base UI Field, Base UI Input and Button.",
    );
  });
  it("adds no absence commentary for native components", () => {
    expect(sentenceFor("panel-header")).toBe("");
  });
  it("links Buzz components internally and Base UI externally", () => {
    expect(baseUiBackingSentence("composer")).toContainEqual({
      kind: "link",
      value: "Button",
      href: "/design/components/button",
      external: false,
    });
    expect(baseUiBackingSentence("button")).toContainEqual({
      kind: "link",
      value: "Base UI Button",
      href: "https://base-ui.com/react/components/button",
      external: true,
    });
  });
  it("keeps every nonempty note concise and punctuated", () => {
    for (const component of COMPONENTS) {
      const sentence = sentenceFor(component.slug);
      if (!sentence) continue;
      expect(sentence.startsWith("Inherits ")).toBe(true);
      expect(sentence.endsWith(".")).toBe(true);
      expect(sentence).not.toContain("..");
      expect(sentence).not.toContain("  ");
      expect(sentence).not.toContain("through");
    }
  });
});
