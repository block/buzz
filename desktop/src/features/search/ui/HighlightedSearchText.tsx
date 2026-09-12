import * as React from "react";

import { splitSearchMatches } from "@/features/search/lib/searchMatch";
import { SEARCH_MATCH_HIGHLIGHT_CLASS } from "@/shared/lib/searchHighlightStyle";

export function HighlightedSearchText({
  query,
  text,
}: {
  query: string;
  text: string;
}) {
  return splitSearchMatches(text, query).map((part) =>
    part.isMatch ? (
      <mark
        className={SEARCH_MATCH_HIGHLIGHT_CLASS}
        data-search-match="true"
        key={part.key}
      >
        {part.text}
      </mark>
    ) : (
      <React.Fragment key={part.key}>{part.text}</React.Fragment>
    ),
  );
}
