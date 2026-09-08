import { createFileRoute } from "@tanstack/react-router";

import { ComposerPage } from "@/features/design-system/ui/ComposerPage";

export const Route = createFileRoute("/design/composer")({
  component: ComposerPage,
});
