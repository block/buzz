import { ProjectsView } from "@/features/projects/ui/ProjectsView";
import { TopChromeBackdrop } from "@/shared/ui/TopChromeBackdrop";

export function ProjectsScreen() {
  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <TopChromeBackdrop />
      <ProjectsView />
    </div>
  );
}
