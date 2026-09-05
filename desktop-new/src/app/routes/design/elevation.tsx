import { createFileRoute } from "@tanstack/react-router";

import { ElevationPage } from "@/features/design-system/ui/ElevationPage";

export const Route = createFileRoute("/design/elevation")({
  component: ElevationPage,
});
