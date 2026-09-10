import { cn } from "@/shared/lib/cn";
import "./TypingDots.css";

/** Decorative typing animation; the containing control owns its status label. */
export function TypingDots({
  className,
  dotClassName,
}: {
  className?: string;
  dotClassName?: string;
}) {
  return (
    <span
      aria-hidden="true"
      className={cn("inline-flex items-center gap-1", className)}
    >
      {[0, 1, 2].map((index) => (
        <span
          key={index}
          className={cn(
            "typing-dot h-1.5 w-1.5 rounded-full bg-muted-foreground",
            dotClassName,
          )}
          style={{ animationDelay: `${index * 120}ms` }}
        />
      ))}
    </span>
  );
}
