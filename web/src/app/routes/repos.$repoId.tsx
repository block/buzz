import { createFileRoute } from "@tanstack/react-router";
import { RepoDetailPage } from "@/features/repos/ui/RepoDetailPage";

export const Route = createFileRoute("/repos/$repoId")({
  component: RepoDetailPage,
});
