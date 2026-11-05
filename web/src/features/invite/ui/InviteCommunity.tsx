import * as React from "react";
import { relayHttpUrl } from "@/shared/lib/relay-url";

/** Keep custom names; shorten only known hosted community suffixes. */
export function communityName(name: unknown, host: string): string {
  if (
    typeof name === "string" &&
    name.trim() &&
    name !== "Buzz Relay" &&
    name !== host
  )
    return name;
  for (const suffix of [
    ".communities.buzz.xyz",
    ".communities.public.builderlabstaging.xyz",
  ]) {
    if (host.endsWith(suffix) && host.length > suffix.length)
      return host.slice(0, -suffix.length);
  }
  return host;
}

/** Public community metadata is decorative; failures never prevent joining. */
export function InviteCommunity({
  relay,
  host,
}: {
  relay: string;
  host: string;
}) {
  const [metadata, setMetadata] = React.useState<{
    name?: unknown;
    icon?: string;
  }>({});
  const [loaded, setLoaded] = React.useState(false);
  React.useEffect(() => {
    const controller = new AbortController();
    setMetadata({});
    setLoaded(false);
    fetch(relayHttpUrl(relay), {
      headers: { Accept: "application/nostr+json" },
      signal: controller.signal,
    })
      .then(async (response) => {
        if (!response.ok) return;
        const value = await response.json();
        if (!controller.signal.aborted && value && typeof value === "object") {
          setMetadata({
            name: value.name,
            icon: typeof value.icon === "string" ? value.icon : undefined,
          });
        }
      })
      .catch(() => {
        /* The hostname remains available when metadata cannot load. */
      });
    return () => controller.abort();
  }, [relay]);
  const clipId = React.useId();
  return (
    <div className="invite-community">
      <svg
        width="0"
        height="0"
        aria-hidden="true"
        style={{ position: "absolute" }}
      >
        <defs>
          <clipPath id={clipId} clipPathUnits="objectBoundingBox">
            <path d="M .5 0 C .93 0 1 .07 1 .5 C 1 .93 .93 1 .5 1 C .07 1 0 .93 0 .5 C 0 .07 .07 0 .5 0 Z" />
          </clipPath>
        </defs>
      </svg>
      {metadata.icon ? (
        <img
          className="invite-community-icon"
          style={{
            display: loaded ? undefined : "none",
            clipPath: `url(#${clipId})`,
          }}
          src={metadata.icon}
          alt=""
          onLoad={() => setLoaded(true)}
          onError={() => setLoaded(false)}
        />
      ) : null}
      <p>{communityName(metadata.name, host)}</p>
    </div>
  );
}
