import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { validateReviewSearch } from "@/features/review-canvas/lib/reviewRoute";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const ReviewCanvasScreen = React.lazy(async () => {
  const module = await import("@/features/review-canvas/ui/ReviewCanvasScreen");
  return { default: module.ReviewCanvasScreen };
});

export const Route = createFileRoute("/channels/$channelId/review/$artifactId")(
  {
    validateSearch: validateReviewSearch,
    component: ReviewRouteComponent,
  },
);

function ReviewRouteComponent() {
  const { channelId, artifactId } = Route.useParams();
  const search = Route.useSearch();

  return (
    <React.Suspense
      fallback={<ViewLoadingFallback includeHeader kind="channel" />}
    >
      <ReviewCanvasScreen
        artifactId={artifactId}
        channelId={channelId}
        search={search}
      />
    </React.Suspense>
  );
}
