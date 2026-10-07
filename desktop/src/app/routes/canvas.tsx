import { createFileRoute } from "@tanstack/react-router";

import { ReviewInboxScreen } from "@/features/review-canvas/ui/ReviewInboxScreen";

export const Route = createFileRoute("/canvas")({
  component: CanvasRouteComponent,
});

function CanvasRouteComponent() {
  return <ReviewInboxScreen />;
}
