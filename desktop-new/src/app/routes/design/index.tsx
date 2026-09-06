import { createFileRoute } from "@tanstack/react-router";

import { OverviewPage } from "@/features/design-system/ui/OverviewPage";

export const Route = createFileRoute("/design/")({ component: OverviewPage });
