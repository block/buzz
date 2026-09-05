import { createFileRoute } from "@tanstack/react-router";

import { GlassPage } from "@/features/design-system/ui/GlassPage";

export const Route = createFileRoute("/design/glass")({
  component: GlassPage,
});
