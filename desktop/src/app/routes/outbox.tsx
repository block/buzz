import { createFileRoute } from "@tanstack/react-router";

import { OutboxScreen } from "@/features/outbox/ui/OutboxScreen";

export const Route = createFileRoute("/outbox")({
  component: OutboxScreen,
});
