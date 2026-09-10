import { createFileRoute } from "@tanstack/react-router";
import { ComponentDetailPage } from "@/features/design-system/ui/ComponentDetailPage";

export const Route = createFileRoute("/design/components_/$component")({
  component: ComponentRoute,
});

function ComponentRoute() {
  const { component } = Route.useParams();
  return <ComponentDetailPage slug={component} />;
}
