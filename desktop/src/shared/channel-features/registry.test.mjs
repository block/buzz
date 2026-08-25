import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";
import { CircleDot, FileText, Hash, Lock } from "lucide-react";

import { registerBuiltinChannelFeatures } from "./builtins.ts";
import {
  __resetChannelFeatureRegistryForTests,
  channelGlyph,
  classifyChannel,
  getChannelPlugins,
  registerChannelFeature,
} from "./registry.ts";

describe("built-in channel-feature plugins", () => {
  beforeEach(() => {
    __resetChannelFeatureRegistryForTests();
    registerBuiltinChannelFeatures();
  });

  it("registers exactly the 4 built-in plugins, in priority order", () => {
    const plugins = getChannelPlugins();
    assert.deepEqual(
      plugins.map((p) => p.id),
      ["dm", "private-channel", "forum", "stream"],
    );
  });

  it("re-registering is a no-op (idempotent)", () => {
    registerBuiltinChannelFeatures();
    assert.equal(getChannelPlugins().length, 4);
  });

  const cases = [
    {
      name: "a dm channel",
      channel: { channelType: "dm", visibility: "open" },
      pluginId: "dm",
      glyph: CircleDot,
    },
    {
      name: "a private stream channel",
      channel: { channelType: "stream", visibility: "private" },
      pluginId: "private-channel",
      glyph: Lock,
    },
    {
      name: "an open forum channel",
      channel: { channelType: "forum", visibility: "open" },
      pluginId: "forum",
      glyph: FileText,
    },
    {
      name: "a plain open stream channel",
      channel: { channelType: "stream", visibility: "open" },
      pluginId: "stream",
      glyph: Hash,
    },
  ];

  for (const { name, channel, pluginId, glyph } of cases) {
    it(`classifies ${name} as "${pluginId}", matching ChatHeader's old ChannelIcon cascade`, () => {
      const binding = classifyChannel(channel);
      assert.ok(binding, "expected a non-null binding");
      assert.equal(binding.pluginId, pluginId);
      assert.equal(channelGlyph(channel), glyph);
    });
  }

  it("dm takes precedence over private (a dm can't be classified as private-channel)", () => {
    const binding = classifyChannel({
      channelType: "dm",
      visibility: "private",
    });
    assert.equal(binding.pluginId, "dm");
  });

  it("private takes precedence over forum", () => {
    const binding = classifyChannel({
      channelType: "forum",
      visibility: "private",
    });
    assert.equal(binding.pluginId, "private-channel");
  });
});

describe("registerChannelFeature", () => {
  beforeEach(() => {
    __resetChannelFeatureRegistryForTests();
  });

  it("classifies against an empty registry as null", () => {
    assert.equal(
      classifyChannel({ channelType: "stream", visibility: "open" }),
      null,
    );
  });

  it("sorts by priority, ties keeping registration order", () => {
    registerChannelFeature({ id: "b", priority: 1, parseBinding: () => true });
    registerChannelFeature({ id: "a", priority: 0, parseBinding: () => true });
    registerChannelFeature({ id: "c", priority: 1, parseBinding: () => true });
    assert.deepEqual(
      getChannelPlugins().map((p) => p.id),
      ["a", "b", "c"],
    );
  });

  it("warns and ignores a duplicate id instead of throwing", () => {
    registerChannelFeature({ id: "dup", parseBinding: () => true });
    assert.doesNotThrow(() =>
      registerChannelFeature({ id: "dup", parseBinding: () => null }),
    );
    assert.equal(getChannelPlugins().length, 1);
  });
});
