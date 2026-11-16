import buzzMark from "@/assets/invite/mark.svg";
import "@/features/invite/ui/invite-page.css";
import { InviteCommunity } from "@/features/invite/ui/InviteCommunity";
import {
  BUZZ_RELEASES_URL,
  detectBuzzDownloadPlatform,
  resolveBuzzDownloadUrlForPlatform,
} from "@/shared/lib/buzz-download";
import { relayWsUrl } from "@/shared/lib/relay-url";
import { Button } from "@/shared/ui/button";
import * as React from "react";
import { conversationAppLink, opensAppOnLoad } from "../conversation-link";

/**
 * Landing page for a shared conversation link (`/channel/<id>[/<event>]`).
 * It reads nothing from the relay beyond public community metadata: the
 * installed app opens the conversation and checks access itself.
 */
export function ConversationLinkPage({
  channelId,
  messageId,
}: {
  channelId: string;
  messageId?: string;
}) {
  const relay = relayWsUrl();
  const host = relay.replace(/^wss?:\/\//, "");
  const appLink = conversationAppLink({
    channelId,
    messageId,
    search: window.location.search,
  });
  const [downloadUrl, setDownloadUrl] = React.useState(BUZZ_RELEASES_URL);

  React.useEffect(() => {
    let active = true;
    detectBuzzDownloadPlatform(navigator).then(async (platform) => {
      if (active && appLink && opensAppOnLoad(platform.operatingSystem))
        window.location.href = appLink;
      const url = await resolveBuzzDownloadUrlForPlatform(platform);
      if (active) setDownloadUrl(url);
    });
    return () => {
      active = false;
    };
  }, [appLink]);

  return (
    <div className="invite-page">
      <img className="invite-mark" src={buzzMark} alt="Buzz" />
      <div className="invite-stack">
        <div className="invite-card">
          <h1>{appLink ? "Open in Buzz" : "This link isn't valid"}</h1>
          <p className="invite-intro">
            {appLink
              ? `This ${messageId ? "message" : "conversation"} is in a Buzz community.`
              : "Check the link, or ask for a new one."}
          </p>
          <InviteCommunity relay={relay} host={host} />
          {appLink ? (
            <div className="invite-actions">
              <Button
                asChild
                className="h-10 w-full bg-black text-white hover:bg-black/90 focus-visible:ring-black"
              >
                <a href={appLink}>Open in Buzz</a>
              </Button>
            </div>
          ) : null}
        </div>
        <p className="invite-download">
          Don&apos;t have the app?{" "}
          <a
            className="ml-1 font-medium text-black underline-offset-4 hover:text-black/70 hover:underline focus-visible:underline"
            href={downloadUrl}
            rel="noreferrer"
            target="_blank"
          >
            Download Buzz
          </a>
        </p>
      </div>
    </div>
  );
}
