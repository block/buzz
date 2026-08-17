import { cn } from "@/shared/lib/cn";

export function DrawerPanelIcon({
  className,
  side,
}: {
  className?: string;
  side: "left" | "right";
}) {
  return (
    <svg
      aria-hidden="true"
      className={cn("h-4 w-auto shrink-0", className)}
      fill="none"
      height="22"
      viewBox="0 0 24 22"
      width="24"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect
        height="20"
        rx="5"
        stroke="currentColor"
        strokeWidth="2"
        width="22"
        x="1"
        y="1"
      />
      <rect
        className="transition-transform duration-200 ease-out motion-reduce:transition-none"
        fill="currentColor"
        height="14"
        rx="3"
        style={{
          transform: side === "right" ? "translateX(10px)" : "translateX(0)",
        }}
        width="6"
        x="4"
        y="4"
      />
    </svg>
  );
}
