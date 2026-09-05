import { createFileRoute } from "@tanstack/react-router";

import { RadiusPage } from "@/features/design-system/ui/RadiusPage";

export const Route = createFileRoute("/design/radius")({
  component: RadiusPage,
});
