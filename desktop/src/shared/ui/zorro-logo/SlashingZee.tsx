import { ZorroHat } from "./ZorroHat";

/**
 * The compositor-friendly Zorro loading mark. Keeping the animation on an
 * HTML wrapper lets WebKit animate it while startup work occupies the main
 * thread. Reduced-motion styling leaves the mark static.
 */
export function SlashingZee({ className }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={[
        "zorro-hat",
        "zorro-loader-mark",
        "relative",
        "aspect-square",
        className,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <ZorroHat className="block h-full w-full" />
    </div>
  );
}
