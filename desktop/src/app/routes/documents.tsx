import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const DocumentsScreen = React.lazy(async () => {
  const module = await import("@/features/documents/ui/DocumentsScreen");
  return { default: module.DocumentsScreen };
});

export const Route = createFileRoute("/documents")({
  component: DocumentsRouteComponent,
});

function DocumentsRouteComponent() {
  usePreviewFeatureWarning("documents");
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="documents" />}>
      <DocumentsScreen />
    </React.Suspense>
  );
}
