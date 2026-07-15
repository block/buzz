import buzzAppIcon from "@/assets/app-icon@3x.png";
import { relayWsUrl } from "@/shared/lib/relay-url";
import { Button } from "@/shared/ui/button";
import * as React from "react";

const DOWNLOAD_URL = "https://github.com/block/buzz/releases/latest";
type Terms = { url: string; privacy_url?: string; version: string };

/** Landing page for a community invite link (`/invite/<code>`). */
export function InvitePage({ code }: { code: string }) {
  const relay = relayWsUrl();
  const host = relay.replace(/^wss?:\/\//, "");
  const [terms, setTerms] = React.useState<Terms | null | undefined>(undefined);
  const [ageConfirmed, setAgeConfirmed] = React.useState(false);
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
            age_confirmed: ageConfirmed,
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
    terms === undefined || opening || (terms !== null && !ageConfirmed);
  return (
    <div
      className="flex flex-1 flex-col items-center justify-center px-4 py-16 text-center"
      style={{
        backgroundImage: "linear-gradient(180deg, #D7D72E 0%, #D7E7F6 100%)",
      }}
    >
      <div className="w-full max-w-xl space-y-4">
        <div className="flex w-full flex-col items-center rounded-3xl bg-white px-6 py-10 sm:px-12 sm:py-12">
          <div
            className="h-12 w-12 overflow-hidden bg-black"
            style={{ borderRadius: "22.37%" }}
          >
            <img alt="Buzz" className="h-full w-full" src={buzzAppIcon} />
          </div>
          <h1 className="mt-4 text-2xl font-semibold tracking-tight text-black">
            You&apos;re invited to
          </h1>
          <p className="mt-9 font-mono text-lg text-black/70">{host}</p>

          {terms && (
            <label className="mt-9 flex max-w-md cursor-pointer items-start gap-3 text-left text-sm text-black/70">
              <input
                className="mt-0.5 h-4 w-4 accent-black"
                type="checkbox"
                checked={ageConfirmed}
                onChange={(event) => setAgeConfirmed(event.target.checked)}
              />
              <span>I confirm that I am at least 18 years old.</span>
            </label>
          )}
          <div className={terms ? "mt-5" : "mt-9"}>
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
          {terms && (
            <p className="mt-4 max-w-md text-xs text-black/60">
              By accepting this invite, you agree to the{" "}
              <a
                className="text-black underline-offset-4 hover:text-black/70 hover:decoration-current hover:underline focus-visible:underline"
                href={terms.url}
                target="_blank"
                rel="noreferrer"
              >
                Terms of Service
              </a>
              {terms.privacy_url && (
                <>
                  {" "}
                  and{" "}
                  <a
                    className="text-black underline-offset-4 hover:text-black/70 hover:decoration-current hover:underline focus-visible:underline"
                    href={terms.privacy_url}
                    target="_blank"
                    rel="noreferrer"
                  >
                    Privacy Policy
                  </a>
                </>
              )}
              .
            </p>
          )}
        </div>
        <p className="flex h-[3.125rem] items-center justify-center rounded-2xl bg-white text-sm text-black/60">
          Don&apos;t have the app?{" "}
          <a
            className="ml-1 font-medium text-black underline-offset-4 hover:text-black/70 hover:decoration-current hover:underline focus-visible:underline"
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
