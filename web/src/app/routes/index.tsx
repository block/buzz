import { createFileRoute } from "@tanstack/react-router";
import { ReposPage } from "@/features/repos/ui/ReposPage";

export const Route = createFileRoute("/")({
  component: ReposPage,
});
