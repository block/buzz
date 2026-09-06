import { describe, expect, it } from "vitest";
import { projectChannelWindow } from "./channelWindowProjection";

const event = (kind: number, id: string, content = "content") => ({
  id,
  pubkey: "author",
  content,
  created_at: 100,
  kind,
  tags: [["h", "channel"]],
});

describe("projectChannelWindow", () => {
  it("keeps protocol overlays and system events out of the transcript", () => {
    const projection = projectChannelWindow([
      event(40099, "system", '{"type":"channel_created"}'),
      event(9, "message", "Hello"),
      event(39005, "summary", '{"reply_count":2}'),
      event(39006, "bounds", '{"has_more":false,"next_cursor":null}'),
    ]);

    expect(projection.messages.map(({ id }) => id)).toEqual(["message"]);
    expect(projection.systemEvents.map(({ id }) => id)).toEqual(["system"]);
    expect(projection.threadSummaries.map(({ id }) => id)).toEqual(["summary"]);
    expect(projection.bounds?.id).toBe("bounds");
  });

  it("rejects an ambiguous window with multiple bounds events", () => {
    expect(() =>
      projectChannelWindow([event(39006, "one"), event(39006, "two")]),
    ).toThrow("more than one bounds");
  });
});
