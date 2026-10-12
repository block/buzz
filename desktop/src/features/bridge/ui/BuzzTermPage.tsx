// Web "Buzz term" page (route /buzz-term): the remote terminal on the member's desktop, or, while it isn't available,
// the exact steps to turn it on. Respects the desktop's Settings → Buzz Term (shell vs herdr, default session) via the heartbeat.
import { useIdentityQuery } from "@/shared/api/hooks";
import { useBridgeAgents } from "../useBridgeAgents";
import { whyTermUnavailable } from "../remoteHosts";
import { BUZZ_TERM_UNAVAILABLE, useRemoteHostsFor, useWebRemoteNav } from "./WebRemoteSettings";
import { RemotePanel } from "./RemotePanel";

export function BuzzTermPage() {
  const pubkey = useIdentityQuery().data?.pubkey;
  const b = useBridgeAgents(pubkey);
  const nav = useWebRemoteNav();
  const { hosts, reachOf } = useRemoteHostsFor(b);
  // Usable as soon as the desktop advertises terminals and answers on the tailnet; the per-device switch in web Settings
  // no longer has to be found and flipped first (it can still be used to switch the nav entry off).
  const ready = hosts.find((h) => h.supports.terminal && !h.stale && reachOf(h) !== "unreachable");
  const why = whyTermUnavailable(hosts[0], !!b.heartbeat?.remote, hosts[0] ? reachOf(hosts[0]) : "unknown");
  const c = BUZZ_TERM_UNAVAILABLE;
  const usable = nav.terminal || !!ready;
  return (
    <div className="flex-1 overflow-y-auto" data-testid="buzz-term-page">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-3 p-6">
        <h1 className="text-xl font-semibold" data-testid="buzz-term-title">
          {usable ? "Buzz term" : c.title}
        </h1>
        {usable && b.available ? (
          <RemotePanel bridge={b} signJson={b.signJson} />
        ) : (
          <div data-testid="remote-unavailable-page" data-feature="term">
            <p className="text-sm text-muted-foreground">{c.summary}</p>
            {why && (
              <p className="mt-2 text-sm font-medium" data-testid="buzz-term-why">
                {why}
              </p>
            )}
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
