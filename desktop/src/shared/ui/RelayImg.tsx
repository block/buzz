import * as React from "react";

import { useRelayMediaSrc } from "@/shared/lib/useRelayMediaSrc";

/**
 * <img> for relay media (custom emoji, uploads). Desktop: renders the URL as-is (the local proxy authenticates).
 * Web: relay media needs an Authorization header a plain <img> can't send, so the bytes are fetched with the shared signed
 * token and shown from a blob: URL; until then an empty box of the same size keeps the layout steady.
 */
export function RelayImg({ src, alt, className, ...rest }: React.ImgHTMLAttributes<HTMLImageElement>) {
  const media = useRelayMediaSrc(typeof src === "string" ? src : undefined);
  if (!media.src) return <span aria-label={alt} className={className} data-relay-img={media.state} role="img" style={{ display: "inline-block" }} />;
  return <img alt={alt} className={className} src={media.src} {...rest} />;
}
