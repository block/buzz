import { FlowStudioView } from "@/features/flow-studio/ui/FlowStudioView";

export function FlowStudioScreen() {
  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <FlowStudioView />
    </div>
  );
}
