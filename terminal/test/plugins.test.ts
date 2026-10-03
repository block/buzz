import assert from "node:assert/strict";
import test from "node:test";
import { Plugins } from "../src/plugins.ts";

test("plugin unload removes subscriptions, commands, views, renderers, and runs cleanup once", () => {
  let subscribed = 0,
    cleaned = 0;
  const plugins = new Plugins(() => {
    subscribed++;
    return () => {
      subscribed--;
    };
  });
  plugins.load({
    id: "test",
    activate(ctx) {
      ctx.command({ name: "test", description: "test", run() {} });
      ctx.view({ id: "test", title: "test", entries: () => [] });
      ctx.renderer(9, () => "test");
      ctx.onChange(() => {});
      ctx.onDispose(() => {
        cleaned++;
      });
    },
  });
  assert.equal(subscribed, 1);
  plugins.unload("test");
  plugins.unload("test");
  assert.equal(subscribed, 0);
  assert.equal(cleaned, 1);
  assert.equal(
    plugins.commands.size + plugins.views.size + plugins.renderers.size,
    0,
  );
});

test("failed activation rolls back without removing another plugin's contribution", () => {
  const plugins = new Plugins(() => () => {});
  plugins.load({
    id: "first",
    activate(ctx) {
      ctx.renderer(9, () => "first");
    },
  });
  assert.throws(
    () =>
      plugins.load({
        id: "second",
        activate(ctx) {
          ctx.command({ name: "temp", description: "temporary", run() {} });
          ctx.renderer(9, () => "second");
        },
      }),
    /Duplicate/,
  );
  assert.equal(plugins.commands.size, 0);
  assert.equal(plugins.renderers.get(9)?.({} as never), "first");
});
