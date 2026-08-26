import { ExternalLink, Globe } from "lucide-react";
import * as React from "react";

import type { ChannelWebsite } from "@/features/channels/lib/channelWebsites";
import {
  channelWebsiteTabLabel,
  isBlockedEmbedLocation,
} from "@/features/channels/lib/channelWebsites";
import { RemoteEmbedFrame } from "@/features/channels/ui/RemoteEmbedFrame";
import { Button } from "@/shared/ui/button";

type ChannelWebsitePaneProps = {
  website: ChannelWebsite;
};

export function ChannelWebsitePane({ website }: ChannelWebsitePaneProps) {
  const label = channelWebsiteTabLabel(website);
  const [embedBlocked, setEmbedBlocked] = React.useState(false);

  const handleFrameLoad = React.useCallback(
    (event: React.SyntheticEvent<HTMLIFrameElement>) => {
      try {
        const href = event.currentTarget.contentWindow?.location.href;
        if (isBlockedEmbedLocation(href)) {
          setEmbedBlocked(true);
        }
      } catch {
        // Cross-origin document: the site allowed the iframe.
      }
    },
    [],
  );

  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      data-testid="channel-website-pane"
    >
      {embedBlocked ? (
        <div
          className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 px-8 text-center"
          data-testid="channel-website-embed-blocked"
        >
          <Globe className="h-8 w-8 text-muted-foreground" />
          <div className="space-y-1">
            <p className="text-base font-medium">{label}</p>
            <p className="max-w-md text-sm text-muted-foreground">
              This site does not allow embedding in Buzz. Open it in a browser
              tab instead.
            </p>
          </div>
          <Button asChild size="sm" type="button">
            <a href={website.url} rel="noopener noreferrer" target="_blank">
              <ExternalLink className="h-3.5 w-3.5" />
              Open {label}
            </a>
          </Button>
        </div>
      ) : (
        <RemoteEmbedFrame
          onLoad={handleFrameLoad}
          src={website.url}
          testId="channel-website-frame"
          title={label}
        />
      )}
    </div>
  );
}
