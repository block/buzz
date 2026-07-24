import * as React from "react";

import { Markdown } from "@/shared/ui/markdown";
import { parseImetaTags } from "@/shared/ui/markdown/parseImeta";

/**
 * Renders project event content with the same link and media support as
 * messages while retaining NIP-92 attachment metadata from the source event.
 */
export function ProjectRichContent({
  className = "text-sm",
  content,
  tags,
}: {
  className?: string;
  content: string;
  tags?: string[][];
}) {
  const imetaByUrl = React.useMemo(
    () => (tags ? parseImetaTags(tags) : undefined),
    [tags],
  );

  return (
    <Markdown className={className} content={content} imetaByUrl={imetaByUrl} />
  );
}
