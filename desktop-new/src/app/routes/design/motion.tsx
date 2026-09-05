import { createFileRoute } from "@tanstack/react-router";

import { MotionPage } from "@/features/design-system/ui/MotionPage";

export const Route = createFileRoute("/design/motion")({
  component: MotionPage,
});
