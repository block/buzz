import { createFileRoute, notFound } from "@tanstack/react-router";
import { CATALOG } from "@/features/design-system/catalog/registry";
import {
  ComponentNotFound,
  ComponentPage,
} from "@/features/design-system/ui/ComponentPage";

// The underscore keeps the URL nested without rendering the entire catalog above it.
export const Route = createFileRoute("/design/components_/$componentId")({
  beforeLoad: ({ params }) => {
    const entry = CATALOG.find(
      (candidate) => candidate.id === params.componentId,
    );
    if (!entry) throw notFound();
    return { entry };
  },
  component: FocusedComponent,
  notFoundComponent: ComponentNotFound,
});

function FocusedComponent() {
  const { entry } = Route.useRouteContext();
  return <ComponentPage key={entry.id} entry={entry} />;
}
