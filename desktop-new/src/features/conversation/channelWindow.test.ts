import { describe, expect, it } from "vitest";
import { mergeMessages, projectChannelWindow } from "./channelWindow";

const event = (
  kind: number,
  id: string,
  content = "content",
  createdAt = 100,
) => ({
  id,
  pubkey: "author",
  content,
  created_at: createdAt,
  kind,
  tags:
    kind === 39006
      ? [
          ["h", "channel"],
          ["d", "channel:head"],
        ]
      : [["h", "channel"]],
});

describe("projectChannelWindow", () => {
  it("keeps protocol overlays and system events out of the transcript", () => {
    const projection = projectChannelWindow(
      [
        event(40099, "system", '{"type":"channel_created"}'),
        event(9, "message", "Hello"),
        event(39005, "summary", '{"reply_count":2}'),
        event(39006, "bounds", '{"has_more":false,"next_cursor":null}'),
      ],
      { channelId: "channel" },
    );

    expect(projection.messages.map(({ id }) => id)).toEqual(["message"]);
    expect(projection.systemEvents.map(({ id }) => id)).toEqual(["system"]);
    expect(projection.threadSummaries.map(({ id }) => id)).toEqual(["summary"]);
    expect(projection.bounds).toEqual({ hasMore: false, nextCursor: null });
  });

  it("rejects ambiguous or internally inconsistent bounds", () => {
    expect(() =>
      projectChannelWindow(
        [
          event(39006, "one", '{"has_more":false,"next_cursor":null}'),
          event(39006, "two", '{"has_more":false,"next_cursor":null}'),
        ],
        { channelId: "channel" },
      ),
    ).toThrow("more than one bounds");
    expect(() =>
      projectChannelWindow(
        [event(39006, "bounds", '{"has_more":true,"next_cursor":null}')],
        { channelId: "channel" },
      ),
    ).toThrow("require next_cursor");
    expect(() =>
      projectChannelWindow([event(9, "message")], { channelId: "channel" }),
    ).toThrow("missing bounds");
  });

  it("uses the event ID as the same-second ordering tie-breaker", () => {
    const projection = projectChannelWindow(
      [
        event(9, "b", "second", 100),
        event(9, "a", "first", 100),
        event(39006, "bounds", '{"has_more":false,"next_cursor":null}'),
      ],
      { channelId: "channel" },
    );

    expect(projection.messages.map(({ id }) => id)).toEqual(["a", "b"]);
  });
});

describe("mergeMessages", () => {
  it("deduplicates the live overlay and keeps total relay order", () => {
    const result = mergeMessages(
      [
        { ...event(9, "b", "b", 100), createdAt: 100 },
        { ...event(9, "c", "c", 101), createdAt: 101 },
      ],
      [
        { ...event(9, "a", "a", 100), createdAt: 100 },
        { ...event(9, "c", "fresh c", 101), createdAt: 101 },
      ],
    );

    expect(result.map(({ id, content }) => [id, content])).toEqual([
      ["a", "a"],
      ["b", "b"],
      ["c", "fresh c"],
    ]);
  });
});
