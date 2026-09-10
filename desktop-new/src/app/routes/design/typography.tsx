import { createFileRoute } from "@tanstack/react-router";

import { TypographyPage } from "@/features/design-system/ui/TypographyPage";

export const Route = createFileRoute("/design/typography")({
  component: TypographyPage,
});
