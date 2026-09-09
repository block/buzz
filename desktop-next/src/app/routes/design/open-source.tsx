import { createFileRoute } from "@tanstack/react-router";
import { OpenSourcePage } from "@/features/design-system/ui/OpenSourcePage";
export const Route = createFileRoute("/design/open-source")({
  component: OpenSourcePage,
});
