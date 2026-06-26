import * as React from "react";
import { toSvg } from "jdenticon";

type BotIdenticonProps = {
  /** The string to generate the identicon from (e.g. the bot's display name or pubkey) */
  value: string;
  /** Size in pixels (default 20) */
  size?: number;
  className?: string;
  "data-testid"?: string;
};

/**
 * Renders a deterministic geometric identicon for a bot instance.
 * Used to visually distinguish numbered bot copies (e.g. Scout::01 vs Scout::02).
 */
export const BotIdenticon = React.memo(function BotIdenticon({
  value,
  size = 20,
  className,
  "data-testid": dataTestid,
}: BotIdenticonProps) {
  const svgHtml = React.useMemo(() => toSvg(value, size), [value, size]);

  return (
    <div
      aria-hidden
      className={className}
      data-testid={dataTestid}
      // biome-ignore lint/security/noDangerouslySetInnerHtml: jdenticon produces safe SVG
      dangerouslySetInnerHTML={{ __html: svgHtml }}
      style={{ width: size, height: size, flexShrink: 0 }}
    />
  );
});
