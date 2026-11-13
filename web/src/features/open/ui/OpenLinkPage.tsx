import buzzMark from "@/assets/invite/mark.svg";
import { BeeField } from "@/features/invite/ui/BeeField";
import "@/features/invite/ui/invite-page.css";
import { buzzLinkFromOpenUrl } from "@/features/open/open-link";
import {
  BUZZ_RELEASES_URL,
  resolveBuzzDownloadUrl,
} from "@/shared/lib/buzz-download";
import { Button } from "@/shared/ui/button";
import * as React from "react";

/** Landing page for `/open/<rest>`: hands `buzz://<rest>` to the app. */
export function OpenLinkPage() {
  const link = buzzLinkFromOpenUrl(
    window.location.pathname,
    window.location.search,
  );
  const launched = React.useRef(false);
  const [downloadUrl, setDownloadUrl] = React.useState(BUZZ_RELEASES_URL);

  React.useEffect(() => {
    if (!link || launched.current) return;
    launched.current = true;
    window.location.href = link;
  }, [link]);

  React.useEffect(() => {
    if (!link) return;
    let active = true;
    resolveBuzzDownloadUrl().then((url) => {
      if (active) setDownloadUrl(url);
    });
    return () => {
      active = false;
    };
  }, [link]);

  return (
    <div className="invite-page">
      <BeeField count={24} sizeScale={0.75} zIndex={-1} />
      <img className="invite-mark" src={buzzMark} alt="Buzz" />
      <div className="invite-stack">
        <div className="invite-card">
          {link ? (
            <>
              <h1>Opening Buzz…</h1>
              <p className="invite-intro">
                If Buzz doesn&apos;t open, use the button below.
              </p>
              <div className="invite-actions">
                <Button
                  asChild
                  className="h-10 w-full bg-black text-white hover:bg-black/90 focus-visible:ring-black"
                >
                  <a href={link}>Open in Buzz</a>
                </Button>
              </div>
            </>
          ) : (
            <>
              <h1>This link doesn&apos;t work</h1>
              <p className="invite-intro">
                Buzz can&apos;t open this address. Check the link and try again.
              </p>
            </>
          )}
        </div>
        {link ? (
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
        ) : null}
      </div>
    </div>
  );
}
