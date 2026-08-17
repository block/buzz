import { AgentStudioView } from "@/features/agent-studio/ui/AgentStudioView";

export function AgentStudioScreen() {
  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <AgentStudioView />
    </div>
  );
}
