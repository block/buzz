import { createFileRoute } from "@tanstack/react-router";

import { ColourPage } from "@/features/design-system/ui/ColourPage";

export const Route = createFileRoute("/design/color")({
  component: ColourPage,
});
