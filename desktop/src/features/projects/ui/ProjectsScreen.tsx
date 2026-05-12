import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import { ProjectsView } from "@/features/projects/ui/ProjectsView";

export function ProjectsScreen() {
  return (
    <>
      <ChatHeader
        description="Repositories and projects on this relay."
        mode="projects"
        overlaysContent
        title="Projects"
      />

      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <ProjectsView />
      </div>
    </>
  );
}
