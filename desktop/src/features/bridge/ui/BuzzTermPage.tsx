// Web "Buzz term" page (route /buzz-term): the remote terminal on the member's desktop, or, while it isn't available,
// the exact steps to turn it on. Respects the desktop's Settings → Buzz Term (shell vs herdr, default session) via the heartbeat.
import { useIdentityQuery } from "@/shared/api/hooks";
import { useBridgeAgents } from "../useBridgeAgents";
import { BUZZ_TERM_UNAVAILABLE, useWebRemoteNav } from "./WebRemoteSettings";
import { RemotePanel } from "./RemotePanel";

export function BuzzTermPage() {
  const pubkey = useIdentityQuery().data?.pubkey;
  const b = useBridgeAgents(pubkey);
  const nav = useWebRemoteNav();
  const c = BUZZ_TERM_UNAVAILABLE;
  return (
    <div className="flex-1 overflow-y-auto" data-testid="buzz-term-page">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-3 p-6">
        <h1 className="text-xl font-semibold" data-testid="buzz-term-title">
          {nav.terminal ? "Buzz term" : c.title}
        </h1>
        {nav.terminal && b.available ? (
          <RemotePanel bridge={b} signJson={b.signJson} />
        ) : (
          <div data-testid="remote-unavailable-page" data-feature="term">
            <p className="text-sm text-muted-foreground">{c.summary}</p>
            <ol className="mt-2 list-decimal space-y-1 pl-5 text-sm" data-testid="remote-unavailable-steps">
              {c.steps.map((t) => (
                <li key={t}>{t}</li>
              ))}
            </ol>
          </div>
        )}
      </div>
    </div>
  );
}
