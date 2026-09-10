import { createFileRoute } from "@tanstack/react-router";
import { GrammarPage } from "@/features/design-system/ui/GrammarPage";

export const Route = createFileRoute("/design/grammar")({
  component: GrammarPage,
});
