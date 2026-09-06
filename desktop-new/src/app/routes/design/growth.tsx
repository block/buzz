import { createFileRoute } from "@tanstack/react-router";

import { GrowthPage } from "@/features/design-system/ui/GrowthPage";

export const Route = createFileRoute("/design/growth")({
  component: GrowthPage,
});
