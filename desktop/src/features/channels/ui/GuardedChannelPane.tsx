import type * as React from "react";

import {
  type MarkdownDocTarget,
  MarkdownDocViewerProvider,
} from "@/shared/ui/markdown/markdownDocViewerContext";

import { ChannelPane } from "./ChannelScreenLazyViews";

/**
 * Hosts the markdown-doc viewer context for the channel pane — the surface
 * that renders the doc auxiliary panel. The forum branch never mounts this
 * wrapper, so forum FileCards keep plain download behavior instead of a
 * dead open-in-viewer click.
 */
export function GuardedChannelPane({
  onOpenMarkdownDoc,
  ...props
}: React.ComponentProps<typeof ChannelPane> & {
  onOpenMarkdownDoc: (doc: MarkdownDocTarget) => void;
}) {
  return (
    <MarkdownDocViewerProvider value={onOpenMarkdownDoc}>
      <ChannelPane {...props} />
    </MarkdownDocViewerProvider>
  );
}
