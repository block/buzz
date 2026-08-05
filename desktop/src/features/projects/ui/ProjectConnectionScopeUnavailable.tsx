import { Button } from "@/shared/ui/button";
import { PROJECT_DETAIL_PANEL_MESSAGE_CLASS } from "./projectPanelStyles";

export function ProjectConnectionScopeUnavailable({
  loading,
}: {
  loading: boolean;
}) {
  if (loading) {
    return (
      <div className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS} role="status">
        Loading this Project's connection scope…
      </div>
    );
  }
  return (
    <div className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}>
      <p>Reconnect to the community to manage Project connections.</p>
      <Button
        className="mt-3"
        onClick={() => window.location.reload()}
        size="sm"
        variant="outline"
      >
        Reload Buzz
      </Button>
    </div>
  );
}
