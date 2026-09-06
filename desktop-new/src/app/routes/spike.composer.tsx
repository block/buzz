import { createFileRoute } from "@tanstack/react-router";

import { ComposerSpikePage } from "@/features/composer-spike/ui/ComposerSpikePage";

export const Route = createFileRoute("/spike/composer")({
  component: ComposerSpikePage,
});
