import { createFileRoute } from "@tanstack/react-router";

import { DesignSystemLayout } from "@/features/design-system/ui/DesignSystemLayout";

export const Route = createFileRoute("/design")({
  component: DesignSystemLayout,
});
