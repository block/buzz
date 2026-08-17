import { createFileRoute } from "@tanstack/react-router";
import { InvitePage } from "@/features/invite/ui/InvitePage";

export const Route = createFileRoute("/invite/$code")({
  component: InvitePageRoute,
});

function InvitePageRoute() {
  const { code } = Route.useParams();
  return <InvitePage code={code} />;
}
