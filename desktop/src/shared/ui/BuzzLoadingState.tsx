import { cn } from "@/shared/lib/cn";
import ZorroLogoAnimation from "@/shared/ui/zorro-logo/ZorroLogoAnimation";

/** Centered, low-emphasis loading state for page and panel fetches. */
export function BuzzLoadingState({
  className,
  fill = false,
  label = "Loading",
}: {
  className?: string;
  fill?: boolean;
  label?: string;
}) {
  return (
    <div
      className={cn(
        "flex w-full items-center justify-center text-muted-foreground/45",
        fill ? "min-h-0 flex-1" : "min-h-[calc(100dvh-7rem)]",
        className,
      )}
      data-testid="buzz-loading-state"
      role="status"
    >
      <ZorroLogoAnimation
        ariaLabel={label}
        className="zorro-logo--pulse"
        fullScreen={false}
        showBackground={false}
        style={{ width: "2rem" }}
        textured={false}
      />
    </div>
  );
}
