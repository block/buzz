import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { cleanup, render, screen } from "@testing-library/react";
import type { Virtualizer } from "@tanstack/react-virtual";

import type { VirtualTimelineRow } from "@/features/messages/lib/buildVirtualTimelineRows";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import { VirtualizedTimelineList } from "./VirtualizedTimelineList";

afterEach(cleanup);

const DAY = 24 * 60 * 60;
const DAY_1 = Math.floor(new Date(2026, 0, 1, 12, 0, 0).getTime() / 1000);
const DAY_2 = DAY_1 + DAY;

// jsdom has no layout engine, so we inject a fake virtualizer that reports all
// rows as "in view". This exercises the real row-dispatch logic — divider vs
// message, flat-index → entry mapping, the absolutely-positioned row wrappers,
// and that the injected renderEntry receives the correct entry — at the DOM
// layer, which is the part VirtualizedTimelineList owns. Pixel measurement and
// scroll math stay on the manual verification pass.
function fakeVirtualizer(
  rows: VirtualTimelineRow[],
): Virtualizer<HTMLDivElement, Element> {
  return {
    getTotalSize: () => rows.length * 64,
    getVirtualItems: () =>
      rows.map((_row, index) => ({
        index,
        key: rows[index].key,
        start: index * 64,
        size: 64,
        end: index * 64 + 64,
        lane: 0,
      })),
    measureElement: () => {},
  } as unknown as Virtualizer<HTMLDivElement, Element>;
}

function messageEntry(id: string, createdAt: number): MainTimelineEntry {
  return {
    message: {
      id,
      createdAt,
      author: "tester",
      time: "",
      body: `body-${id}`,
      depth: 0,
      kind: 9,
    },
    summary: null,
  };
}

// Trivial injected renderer — proves the dispatch without dragging in the heavy
// real MessageRow subtree (emoji-mart, shiki, tiptap). MessageTimeline binds the
// real `renderTimelineEntry` here in production.
function renderEntryStub(entry: MainTimelineEntry) {
  return <span data-testid="entry">{entry.message.body}</span>;
}

function renderIntroStub() {
  return <div data-testid="message-channel-intro">intro</div>;
}

function divider(timestamp: number): VirtualTimelineRow {
  return {
    kind: "day-divider",
    key: `day-${timestamp}`,
    headingTimestamp: timestamp,
    messageIndex: -1,
  };
}

test("renders a day divider row with its formatted label", () => {
  const rows = [divider(DAY_1)];
  render(
    <VirtualizedTimelineList
      entries={[]}
      renderEntry={renderEntryStub}
      scrollMargin={0}
      rows={rows}
      virtualizer={fakeVirtualizer(rows)}
    />,
  );
  assert.equal(screen.getAllByTestId("message-timeline-day-divider").length, 1);
});

test("dispatches message rows to their mapped entry, interleaved with dividers", () => {
  const entries = [messageEntry("a", DAY_1), messageEntry("b", DAY_2)];
  // layout: [div(day1), a, div(day2), b]
  const rows: VirtualTimelineRow[] = [
    divider(DAY_1),
    { kind: "message", key: "a", messageIndex: 0 },
    divider(DAY_2),
    { kind: "message", key: "b", messageIndex: 1 },
  ];
  const { container } = render(
    <VirtualizedTimelineList
      entries={entries}
      renderEntry={renderEntryStub}
      scrollMargin={0}
      rows={rows}
      virtualizer={fakeVirtualizer(rows)}
    />,
  );
  // Two dividers, strictly interleaved, and each message routed to its entry.
  assert.equal(screen.getAllByTestId("message-timeline-day-divider").length, 2);
  assert.ok(screen.getByText("body-a"));
  assert.ok(screen.getByText("body-b"));
  // One absolutely-positioned wrapper per virtual row (4 total).
  assert.equal(container.querySelectorAll("[data-index]").length, 4);
});

test("renders nothing for an empty row list", () => {
  const { container } = render(
    <VirtualizedTimelineList
      entries={[]}
      renderEntry={renderEntryStub}
      scrollMargin={0}
      rows={[]}
      virtualizer={fakeVirtualizer([])}
    />,
  );
  assert.equal(container.querySelectorAll("[data-index]").length, 0);
});

test("renders a message row's wrapper even if its entry is missing (no throw)", () => {
  // A message row whose messageIndex outruns the entries array (transient
  // snapshot skew) must not throw — it renders an empty wrapper.
  const rows: VirtualTimelineRow[] = [
    { kind: "message", key: "ghost", messageIndex: 0 },
  ];
  const { container } = render(
    <VirtualizedTimelineList
      entries={[]}
      renderEntry={renderEntryStub}
      scrollMargin={0}
      rows={rows}
      virtualizer={fakeVirtualizer(rows)}
    />,
  );
  assert.equal(container.querySelectorAll("[data-index]").length, 1);
  assert.equal(screen.queryByTestId("entry"), null);
});

test("positions rows at start minus scrollMargin (content-above offset)", () => {
  // With a scrollMargin of 128px, the first row (virtualItem.start = 0 from the
  // fake virtualizer, since start includes the margin in real usage) must paint
  // at translateY(start - 128). This is the fix for the header/list sandwich:
  // the spacer sits at offsetTop = scrollMargin, so rows subtract it back out.
  const rows: VirtualTimelineRow[] = [divider(DAY_1)];
  const fake = {
    getTotalSize: () => 64,
    getVirtualItems: () => [
      { index: 0, key: rows[0].key, start: 200, size: 64, end: 264, lane: 0 },
    ],
    measureElement: () => {},
  } as unknown as Virtualizer<HTMLDivElement, Element>;

  const { container } = render(
    <VirtualizedTimelineList
      entries={[]}
      renderEntry={renderEntryStub}
      scrollMargin={128}
      rows={rows}
      virtualizer={fake}
    />,
  );
  const wrapper = container.querySelector<HTMLElement>('[data-index="0"]');
  assert.ok(wrapper);
  // 200 - 128 = 72
  assert.match(wrapper.style.transform, /translateY\(72px\)/);
});

test("renders an intro row through the injected intro renderer", () => {
  const rows: VirtualTimelineRow[] = [
    { kind: "intro", key: "channel-intro", messageIndex: -1 },
    divider(DAY_1),
  ];
  render(
    <VirtualizedTimelineList
      entries={[]}
      renderEntry={renderEntryStub}
      renderIntro={renderIntroStub}
      scrollMargin={0}
      rows={rows}
      virtualizer={fakeVirtualizer(rows)}
    />,
  );
  assert.ok(screen.getByTestId("message-channel-intro"));
});
