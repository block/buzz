import { createFileRoute } from "@tanstack/react-router";
import { CompositionsPage } from "@/features/design-system/ui/CompositionsPage";
export const Route = createFileRoute("/design/compositions")({
  component: CompositionsPage,
});
