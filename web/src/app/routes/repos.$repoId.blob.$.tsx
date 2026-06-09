import { createFileRoute } from "@tanstack/react-router";
import { RepoBlobPage } from "@/features/repos/ui/RepoBlobViewer";

export const Route = createFileRoute("/repos/$repoId/blob/$")({
  component: RepoBlobPage,
});
