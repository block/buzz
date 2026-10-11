// Web/mobile Browsers page: the single place for browsers on the web build. It mirrors the desktop's Browsers list over the relay bridge.
import { useChannelsQuery } from "@/features/channels/hooks";
import { BrowserPanel } from "@/features/bridge/ui/BrowserPanel";
import { useBridgeAgents } from "@/features/bridge/useBridgeAgents";
import { useIdentityQuery } from "@/shared/api/hooks";

export function WebBrowsersScreen() {
  const pubkey = useIdentityQuery().data?.pubkey;
  const b = useBridgeAgents(pubkey);
  const channels = (useChannelsQuery().data ?? [])
    .filter((c) => c.channelType !== "dm")
    .map((c) => ({ id: c.id, name: c.name }));
  return (
    <div className="flex-1 overflow-y-auto px-4 py-7 sm:px-6 sm:py-8" data-testid="web-browsers-page">
      <div className="mx-auto w-full max-w-3xl">
        <h1 style={{ fontSize: 20, margin: "0 0 4px" }}>Browsers</h1>
        {!b.available ? null : !b.commander || b.presence === "unknown" ? (
          <p style={{ fontSize: 13, opacity: 0.7 }}>Connecting to your desktop…</p>
        ) : b.presence === "offline" ? (
          <p data-testid="web-browsers-offline" style={{ fontSize: 13 }}>
            Your desktop is offline, so its browsers can't be shown or changed. Start the Buzz desktop app.
          </p>
        ) : null}
        {b.commander && <BrowserPanel bridge={b} channels={channels} commander={b.commander} />}
        {b.commander && !b.commander.supports("browser.open") && b.presence === "online" && (
          <p style={{ fontSize: 13 }}>Update the desktop app to manage browsers from here.</p>
        )}
      </div>
    </div>
  );
}
