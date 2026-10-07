import { createFileRoute } from "@tanstack/react-router";

import { validateReviewSearch } from "@/features/review-canvas/lib/reviewRoute";
import { ReviewCanvasScreen } from "@/features/review-canvas/ui/ReviewCanvasScreen";

export const Route = createFileRoute("/canvas/$channelId/$artifactId")({
  validateSearch: validateReviewSearch,
  component: ReviewRouteComponent,
});

function ReviewRouteComponent() {
  const { channelId, artifactId } = Route.useParams();
  const search = Route.useSearch();

  return (
    <ReviewCanvasScreen
      artifactId={artifactId}
      channelId={channelId}
      search={search}
    />
  );
}
