import { createFileRoute } from "@tanstack/react-router";

import { BrowserSurface } from "@/features/plugins/BrowserSurface";

export const Route = createFileRoute("/plugins/$pluginId/$contributionId")({
  component: PluginBrowserRouteComponent,
});

/**
 * Distinct per plugin+contribution pair — forces `BrowserSurface` to remount
 * on navigation between different plugin routes instead of reusing the
 * instance, since its session/command-tracking refs are not designed to
 * reset in place for a prop change alone (same pattern as community
 * switching in App.tsx). Must vary with *both* params: a key derived from
 * only one would fail to remount when just the other changes.
 */
export function pluginBrowserSurfaceKey(
  pluginId: string,
  contributionId: string,
): string {
  return `${pluginId}:${contributionId}`;
}

/**
 * The actual keyed remount wiring, factored out of the route component so it
 * can be exercised directly (with plain props) without a router context.
 */
export function PluginBrowserRoute({
  pluginId,
  contributionId,
}: {
  pluginId: string;
  contributionId: string;
}) {
  return (
    <BrowserSurface
      key={pluginBrowserSurfaceKey(pluginId, contributionId)}
      contributionId={contributionId}
      pluginId={pluginId}
    />
  );
}

function PluginBrowserRouteComponent() {
  const { pluginId, contributionId } = Route.useParams();
  return (
    <PluginBrowserRoute contributionId={contributionId} pluginId={pluginId} />
  );
}
