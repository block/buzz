import type * as React from "react";

import { MarkdownDetails, MarkdownDetailsSummary } from "./MarkdownDetails";
import { SpoilerInline } from "./SpoilerInline";

/** Components for content a reader reveals: spoilers and collapsible
 * sections (`remarkSpoilers`, `remarkDetails`). */
export function createDisclosureComponents(interactive: boolean) {
  return {
    details: ({
      children,
      ...props
    }: {
      "data-details-key"?: string;
      children?: React.ReactNode;
    }) => (
      <MarkdownDetails
        detailsKey={props["data-details-key"]}
        interactive={interactive}
      >
        {children}
      </MarkdownDetails>
    ),
    summary: MarkdownDetailsSummary,
    spoiler: ({
      children,
      ...props
    }: {
      "data-block-spoiler"?: string;
      children?: React.ReactNode;
    }) => (
      <SpoilerInline
        block={props["data-block-spoiler"] != null}
        interactive={interactive}
      >
        {children}
      </SpoilerInline>
    ),
  };
}
