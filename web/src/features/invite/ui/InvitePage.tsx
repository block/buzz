import buzzAppIcon from "../../../../../desktop/public/app-icon@3x.png";
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
    <div
      className="flex flex-1 flex-col items-center justify-center px-4 py-16 text-center"
      style={{
        backgroundImage: "linear-gradient(180deg, #D7D72E 0%, #D7E7F6 100%)",
      }}
    >
      <div
        className="flex w-full max-w-xl flex-col items-center rounded-3xl bg-white px-6 py-10 sm:px-12 sm:py-12"
        style={{
          boxShadow:
            "0 0 0 1px rgba(0, 0, 0, 0.04), 0 8px 24px rgba(0, 0, 0, 0.04)",
        }}
      >
        <div
          className="h-16 w-16 overflow-hidden bg-black"
          style={{ borderRadius: "22.37%" }}
        >
          <img alt="Buzz" className="h-full w-full" src={buzzAppIcon} />
        </div>
        <h1 className="mt-6 text-2xl font-semibold tracking-tight text-black">
          You&apos;re invited to join
        </h1>
        <p className="mt-1 font-mono text-lg text-foreground">{host}</p>

        <div className="mt-8">
          <Button
            asChild
            className="bg-black text-white hover:bg-black/90 focus-visible:ring-black"
            size="lg"
          >
            <a href={deepLink}>Accept invite in Buzz</a>
          </Button>
        </div>

        <p className="mt-6 text-sm text-muted-foreground">
          Don&apos;t have the app?{" "}
          <a
            className="font-medium text-black underline-offset-4 hover:text-black/70 hover:decoration-current hover:underline focus-visible:underline"
            href={DOWNLOAD_URL}
            rel="noreferrer"
            target="_blank"
          >
            Download it now
          </a>
        </p>
      </div>
    </div>
  );
}
