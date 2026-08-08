import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

type CaretGeometry = {
  caretLeft: number;
  caretMeasurement: "collapsed-range" | "marker";
  caretRight: number;
  caretWidth: number;
  chipRight: number;
  selectionAfterSeparator: boolean;
  selectionInsideChip: boolean;
  spaceLeft: number;
  spaceRight: number;
  spaceWidth: number;
  whiteSpace: string;
};

async function measureMentionCaret(
  input: import("@playwright/test").Locator,
): Promise<CaretGeometry> {
  return input.evaluate((element) => {
    const chip = element.querySelector(".agent-mention-highlight");
    if (!(chip instanceof HTMLElement)) {
      throw new Error("agent mention chip is missing");
    }

    const separator = chip.nextSibling;
    if (!(separator instanceof Text) || separator.data !== " ") {
      throw new Error("U+0020 separator is not the chip's text sibling");
    }

    const selection = window.getSelection();
    if (!selection?.isCollapsed || selection.rangeCount !== 1) {
      throw new Error("composer selection is not a collapsed caret");
    }

    const separatorIndex = Array.prototype.indexOf.call(
      separator.parentNode?.childNodes ?? [],
      separator,
    );
    const selectionAfterSeparator =
      (selection.anchorNode === separator && selection.anchorOffset === 1) ||
      (selection.anchorNode === separator.parentNode &&
        selection.anchorOffset === separatorIndex + 1);

    const spaceRange = document.createRange();
    spaceRange.setStart(separator, 0);
    spaceRange.setEnd(separator, 1);

    const caretRange = selection.getRangeAt(0).cloneRange();
    const chipRect = chip.getBoundingClientRect();
    const spaceRect = spaceRange.getBoundingClientRect();
    let caretRect = caretRange.getBoundingClientRect();
    let caretMeasurement: CaretGeometry["caretMeasurement"] = "collapsed-range";

    // Some engines expose an all-zero rectangle for collapsed ranges. Measure
    // that same live selection with a zero-width inline marker, then restore
    // the separator and caret before returning to the test.
    if (
      caretRect.left === 0 &&
      caretRect.right === 0 &&
      caretRect.width === 0
    ) {
      const marker = document.createElement("span");
      marker.dataset.caretGeometryMarker = "";
      marker.style.display = "inline-block";
      marker.style.width = "0";
      marker.style.height = "1em";
      marker.style.margin = "0";
      marker.style.padding = "0";
      marker.style.border = "0";

      caretRange.insertNode(marker);
      caretRect = marker.getBoundingClientRect();
      caretMeasurement = "marker";
      marker.remove();
      chip.parentNode?.normalize();

      const restoredSeparator = chip.nextSibling;
      if (
        !(restoredSeparator instanceof Text) ||
        restoredSeparator.data !== " "
      ) {
        throw new Error(
          "failed to restore the U+0020 separator after measurement",
        );
      }
      const restoredCaret = document.createRange();
      restoredCaret.setStart(restoredSeparator, 1);
      restoredCaret.collapse(true);
      selection.removeAllRanges();
      selection.addRange(restoredCaret);
    }

    return {
      caretLeft: caretRect.left,
      caretMeasurement,
      caretRight: caretRect.right,
      caretWidth: caretRect.width,
      chipRight: chipRect.right,
      selectionAfterSeparator,
      selectionInsideChip: chip.contains(selection.anchorNode),
      spaceLeft: spaceRect.left,
      spaceRight: spaceRect.right,
      spaceWidth: spaceRect.width,
      whiteSpace: getComputedStyle(element).whiteSpace,
    };
  });
}

async function selectAgentMention(page: import("@playwright/test").Page) {
  await installMockBridge(page, { activePersonaIds: ["builtin:fizz"] });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@Fi");
  await expect(
    page
      .getByTestId("message-composer")
      .getByTestId("mention-autocomplete")
      .getByText("Fizz"),
  ).toBeVisible();
  await input.press("Enter");
  await expect(input).toHaveText("@Fizz ");
  return input;
}

// This Chromium smoke test protects the cross-browser composer invariant:
// autocomplete leaves a rendered separator and the caret outside the chip.
// Native packaged-WebKit acceptance remains separate evidence.
test("composer preserves separator geometry after agent mention autocomplete", async ({
  page,
}, testInfo) => {
  const input = await selectAgentMention(page);
  const geometry = await measureMentionCaret(input);
  testInfo.annotations.push({
    type: "geometry",
    description: JSON.stringify(geometry),
  });

  expect(geometry.whiteSpace).toBe("break-spaces");
  expect(geometry.selectionAfterSeparator).toBe(true);
  expect(geometry.selectionInsideChip).toBe(false);
  expect(geometry.spaceWidth).toBeGreaterThan(0);
  expect(geometry.spaceLeft).toBeCloseTo(geometry.chipRight, 1);
  expect(geometry.caretLeft).toBeCloseTo(geometry.spaceRight, 1);
  expect(geometry.caretRight).toBeGreaterThanOrEqual(geometry.chipRight);
  expect(geometry.caretWidth).toBeLessThanOrEqual(1);
});
