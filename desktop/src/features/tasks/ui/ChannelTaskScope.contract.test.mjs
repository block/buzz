import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const routePath = "src/app/routes/ChannelRouteScreen.tsx";
const tasksPath = "src/features/tasks/ui/TasksScreen.tsx";

test("channel route exposes a toggleable channel-scoped task auxiliary panel", async () => {
  const source = await readFile(routePath, "utf8");

  assert.match(source, /data-testid="toggle-channel-tasks"/);
  assert.match(source, /<ChannelTaskList\s+channelId=\{channelId\}/);
  assert.match(source, /idleAuxiliaryPanel=\{\s*channelTasksPanelOpen/);
  assert.match(
    source,
    /onCloseIdleAuxiliaryPanel=\{\(\) => setChannelTasksPanelChannelId\(null\)\}/,
  );
});

test("global tasks route exposes a channel scope control", async () => {
  const source = await readFile(tasksPath, "utf8");

  assert.match(source, /data-testid="channel-task-scope"/);
  assert.match(source, /All channels/);
  assert.match(source, /channelId=\{selectedChannelId\}/);
});
