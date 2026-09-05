import { createFileRoute } from "@tanstack/react-router";

import { SpacingPage } from "@/features/design-system/ui/SpacingPage";

export const Route = createFileRoute("/design/spacing")({
  component: SpacingPage,
});
