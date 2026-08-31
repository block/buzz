import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const sourcePath = "src/features/projects/ui/ProjectChannelHome.tsx";

test("project home exposes a channel-task toggle in its existing auxiliary slot", async () => {
  const source = await readFile(sourcePath, "utf8");
  assert.match(source, /testId="project-home-channel-tasks-toggle"/);
  assert.match(source, /<ChannelTaskList channelId=\{homeChannel\.id\}/);
  assert.match(source, /channelTasksOpen \? channelTaskPanel : workspaceSheet/);
  assert.match(
    source,
    /idleAuxiliaryOverridesThread=\{\s*channelTasksOpen \|\| workspaceSheetOpen/,
  );
  assert.match(source, /channelTasksOpen\s*\?\s*"Channel tasks"/);
});
