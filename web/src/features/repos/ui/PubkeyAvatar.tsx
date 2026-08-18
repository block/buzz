import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

/** Simple hash of a hex pubkey to a hue value (0-360). */
function pubkeyToHue(hex: string): number {
  let hash = 0;
  for (let i = 0; i < hex.length; i++) {
    hash = (hash * 31 + hex.charCodeAt(i)) | 0;
  }
  return Math.abs(hash) % 360;
}

export function PubkeyAvatar({
  pubkey,
  size = "md",
}: {
  pubkey: string;
  size?: "sm" | "md";
}) {
  const hue = pubkeyToHue(pubkey);
  const sizeClasses = size === "sm" ? "h-6 w-6 text-[10px]" : "h-8 w-8 text-xs";

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          className={`flex items-center justify-center rounded-lg font-medium text-white ${sizeClasses}`}
          style={{ backgroundColor: `hsl(${hue}, 55%, 45%)` }}
        >
          {pubkey.slice(0, 2)}
        </div>
      </TooltipTrigger>
      <TooltipContent>
        <span className="font-mono text-xs">{pubkey}</span>
      </TooltipContent>
    </Tooltip>
  );
}
