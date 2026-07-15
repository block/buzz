import buzzAppIcon from "@/assets/app-icon@3x.png";
import { relayWsUrl } from "@/shared/lib/relay-url";
import { Button } from "@/shared/ui/button";
import * as React from "react";

const DOWNLOAD_URL = "https://github.com/block/buzz/releases/latest";
type Terms = { url: string; version: string };

/** Landing page for a community invite link (`/invite/<code>`). */
export function InvitePage({ code }: { code: string }) {
  const relay = relayWsUrl();
  const host = relay.replace(/^wss?:\/\//, "");
  const [terms, setTerms] = React.useState<Terms | null | undefined>(undefined);
  const [accepted, setAccepted] = React.useState(false);
  const [opening, setOpening] = React.useState(false);

  React.useEffect(() => {
    fetch("/api/invites/config")
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const config = (await response.json()) as { terms?: Terms };
        setTerms(config.terms ?? null);
      })
      .catch(() => setTerms(undefined));
  }, []);

  const openInvite = async () => {
    if (terms && !accepted) return;
    setOpening(true);
    try {
      let receipt: string | undefined;
      if (terms) {
        const response = await fetch("/api/invites/accept-terms", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            code,
            policy_version: terms.version,
            accepted: true,
          }),
        });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        receipt = ((await response.json()) as { receipt: string }).receipt;
      }
      const query = new URLSearchParams({ relay, code });
      if (receipt) query.set("terms_receipt", receipt);
      window.location.href = `buzz://join?${query.toString()}`;
    } finally {
      setOpening(false);
    }
  };

  const disabled =
    terms === undefined || opening || (terms !== null && !accepted);
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
        <p className="mt-1 font-mono text-lg text-black/70">{host}</p>

        {terms && (
          <label className="mt-8 flex max-w-md cursor-pointer items-start gap-3 text-left text-sm text-black/70">
            <input
              className="mt-0.5 h-4 w-4 accent-black"
              type="checkbox"
              checked={accepted}
              onChange={(event) => setAccepted(event.target.checked)}
            />
            <span>
              I agree to this relay operator&apos;s{" "}
              <a
                className="font-medium text-black underline"
                href={terms.url}
                target="_blank"
                rel="noreferrer"
              >
                Terms of Service
              </a>
              .
            </span>
          </label>
        )}

        <div className={terms ? "mt-6" : "mt-8"}>
          {terms === null ? (
            <Button
              asChild
              className="bg-black text-white hover:bg-black/90 focus-visible:ring-black"
              size="lg"
            >
              <a
                href={`buzz://join?relay=${encodeURIComponent(relay)}&code=${encodeURIComponent(code)}`}
              >
                Accept invite in Buzz
              </a>
            </Button>
          ) : (
            <Button
              className="bg-black text-white hover:bg-black/90 focus-visible:ring-black disabled:cursor-not-allowed disabled:bg-black/30 disabled:text-white/70"
              size="lg"
              disabled={disabled}
              onClick={openInvite}
            >
              Accept invite in Buzz
            </Button>
          )}
        </div>
        <p className="mt-6 text-sm text-black/60">
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
