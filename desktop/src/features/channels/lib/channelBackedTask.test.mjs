import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  channelIdFromLink,
  githubRepositoryUrl,
  nestChannels,
  parseChannelBackedTask,
} from "./channelBackedTask.ts";

const CANVAS = `---
buzz_schema: channel-backed-task/v1
task:
  title: "Add status sounds"
  description: "Move playback into Berd Voice."
parent_channel: "buzz://channel/parent"
originating_thread: "buzz://message?channel=parent&id=message"
branch:
  repository: "https://github.com/block/berd"
  name: "jtennant/status-sounds"
---

Experimental task.`;

describe("parseChannelBackedTask", () => {
  it("reads the experiment canvas", () => {
    assert.deepEqual(parseChannelBackedTask(CANVAS), {
      task: {
        title: "Add status sounds",
        description: "Move playback into Berd Voice.",
      },
      parentChannel: "buzz://channel/parent",
      originatingThread: "buzz://message?channel=parent&id=message",
      branch: {
        repository: "https://github.com/block/berd",
        name: "jtennant/status-sounds",
      },
    });
  });

  it("accepts a branchless task", () => {
    const task = parseChannelBackedTask(
      CANVAS.replace(/branch:[\s\S]*?\n---/, "branch: null\n---"),
    );
    assert.equal(task?.branch, null);
  });

  it("ignores ordinary and malformed canvases", () => {
    assert.equal(parseChannelBackedTask("# Notes"), null);
    assert.equal(parseChannelBackedTask("---\nbuzz_schema: [\n---"), null);
    assert.equal(
      parseChannelBackedTask(
        CANVAS.replace('title: "Add status sounds"', 'title: ""'),
      ),
      null,
    );
  });
});

describe("githubRepositoryUrl", () => {
  it("normalizes GitHub repository URLs", () => {
    assert.equal(
      githubRepositoryUrl("https://github.com/block/berd.git"),
      "https://github.com/block/berd",
    );
    assert.equal(githubRepositoryUrl("https://example.com/block/berd"), null);
  });
});

describe("channel-backed task hierarchy", () => {
  const channel = (id) => ({ id, name: id });

  it("reads channel links and recursively nests children", () => {
    assert.equal(channelIdFromLink("buzz://channel/parent"), "parent");
    assert.equal(channelIdFromLink("https://example.com/parent"), null);
    assert.deepEqual(
      nestChannels(
        [channel("grandchild"), channel("parent"), channel("child")],
        new Map([
          ["child", "parent"],
          ["grandchild", "child"],
        ]),
      ).map(({ channel: item, depth }) => [item.id, depth]),
      [
        ["parent", 0],
        ["child", 1],
        ["grandchild", 2],
      ],
    );
  });

  it("keeps missing parents and cycles visible at the top level", () => {
    assert.deepEqual(
      nestChannels(
        [channel("a"), channel("b"), channel("orphan")],
        new Map([
          ["a", "b"],
          ["b", "a"],
          ["orphan", "missing"],
        ]),
      ).map(({ channel: item, depth }) => [item.id, depth]),
      [
        ["orphan", 0],
        ["a", 0],
        ["b", 1],
      ],
    );
  });
});
