import { createFileRoute } from "@tanstack/react-router";

import { VocabularyPage } from "@/features/design-system/ui/VocabularyPage";

export const Route = createFileRoute("/design/vocabulary")({
  component: VocabularyPage,
});
