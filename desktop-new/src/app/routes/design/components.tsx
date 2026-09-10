import { createFileRoute } from "@tanstack/react-router";
import { ComponentsPage } from "@/features/design-system/ui/ComponentsPage";

export const Route = createFileRoute("/design/components")({
  component: ComponentsPage,
});
