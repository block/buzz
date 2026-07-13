import { Download, ExternalLink, Hexagon } from "lucide-react";

import { relayWsUrl } from "@/shared/lib/relay-url";
import { Button } from "@/shared/ui/button";

const DOWNLOAD_URL = "https://github.com/block/buzz/releases/latest";

/**
 * Landing page for a relay invite link (`/invite/<code>`).
 *
 * The code is not validated here — validation happens in the desktop app when
 * the invite is claimed against `POST /api/invites/claim`, signed by the
 * joining key. This page only hands the code off via the `buzz://join` deep
 * link (or tells the visitor where to get the app first).
 */
export function InvitePage({ code }: { code: string }) {
  const relay = relayWsUrl();
  const host = relay.replace(/^wss?:\/\//, "");
  const deepLink = `buzz://join?relay=${encodeURIComponent(relay)}&code=${encodeURIComponent(code)}`;

  return (
    <div className="flex flex-1 flex-col items-center justify-center px-4 py-16 text-center">
      <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-primary/10">
        <Hexagon className="h-8 w-8 text-primary" />
      </div>
      <h1 className="mt-6 text-2xl font-semibold tracking-tight">
        You&apos;re invited to join
      </h1>
      <p className="mt-1 font-mono text-lg text-foreground">{host}</p>
      <p className="mt-3 max-w-md text-sm leading-relaxed text-muted-foreground">
        Buzz is a messaging platform for human–agent collaboration. Accept this
        invite in the desktop app to join the workspace on this relay.
      </p>

      <div className="mt-8 flex flex-col items-center gap-3 sm:flex-row">
        <Button asChild size="lg">
          <a href={deepLink}>
            <ExternalLink className="h-4 w-4" />
            Open in Buzz
          </a>
        </Button>
        <Button asChild variant="outline" size="lg">
          <a href={DOWNLOAD_URL} target="_blank" rel="noreferrer">
            <Download className="h-4 w-4" />
            Download Buzz
          </a>
        </Button>
      </div>

      <p className="mt-6 max-w-sm text-xs leading-relaxed text-muted-foreground">
        Already have Buzz installed? &ldquo;Open in Buzz&rdquo; will launch it
        and accept the invite. Otherwise download the app first, then come back
        and use this link again.
      </p>
    </div>
  );
}
