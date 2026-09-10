import { describe, expect, it } from "vitest";
import { resolveMessagesDestination } from "./resolveMessagesDestination";

describe("resolveMessagesDestination", () => {
  const channels = [{ id: "design" }, { id: "session" }];

  it("opens the first ordinary channel when Messages has no selection", () => {
    expect(
      resolveMessagesDestination(
        { type: "empty" },
        channels,
        new Set(["session"]),
      ),
    ).toEqual({ type: "channel", channelId: "design" });
  });

  it("does not replace an explicit destination", () => {
    expect(
      resolveMessagesDestination(
        { type: "session", channelId: "session" },
        channels,
        new Set(["session"]),
      ),
    ).toEqual({ type: "session", channelId: "session" });
  });

  it("keeps the empty destination when no ordinary channel is available", () => {
    expect(
      resolveMessagesDestination(
        { type: "empty" },
        channels,
        new Set(channels.map(({ id }) => id)),
      ),
    ).toEqual({ type: "empty" });
  });
});
