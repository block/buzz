import { ChannelTaskList } from "@/features/tasks/ui/ChannelTaskList";

/**
 * REG-15: the Tasks route screen — channel tasks for the active community.
 * "Channel task" naming per reflecting D4 Option A. v1 slice: the community's
 * visible tasks (list + create + complete/reopen); the consolidated My-Tasks
 * fan-in (`useMyWorkspaceTasks`) layers on the same commands next.
 */
export function TasksScreen() {
  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-4">
      <header>
        <h1 className="text-lg font-semibold">Channel tasks</h1>
        <p className="text-sm text-muted-foreground">
          Tasks in this community&apos;s channels. Completing a task here is
          visible to every member and on mobile.
        </p>
      </header>
      <ChannelTaskList channelId={null} />
    </div>
  );
}
