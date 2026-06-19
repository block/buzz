import gooseIconMask from "@/shared/assets/goose-icon-mask.png";
import { cn } from "@/shared/lib/cn";

/** Flying goose app mark - silhouette mask tinted with `currentColor`. */
export function GooseIcon({ className = "" }: { className?: string }) {
  return (
    <span
      aria-label="Goose"
      className={cn("inline-block bg-current", className)}
      role="img"
      style={{
        WebkitMaskImage: `url(${gooseIconMask})`,
        maskImage: `url(${gooseIconMask})`,
        WebkitMaskPosition: "center",
        maskPosition: "center",
        WebkitMaskRepeat: "no-repeat",
        maskRepeat: "no-repeat",
        WebkitMaskSize: "contain",
        maskSize: "contain",
      }}
    />
  );
}
