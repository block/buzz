import { createFileRoute } from "@tanstack/react-router";
import { OpenLinkPage } from "@/features/open/ui/OpenLinkPage";

export const Route = createFileRoute("/open/$")({
  component: OpenLinkPage,
});
