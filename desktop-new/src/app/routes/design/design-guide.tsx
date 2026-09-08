import { createFileRoute } from "@tanstack/react-router";
import { SystemDocumentPage } from "@/features/design-system/ui/SystemDocumentPage";

export const Route = createFileRoute("/design/design-guide")({
  component: () => <SystemDocumentPage document="design" />,
});
