import { cn } from "@/shared/lib/cn";

type ShimmerProps = {
  children: string;
  className?: string;
};

export function Shimmer({ children, className }: ShimmerProps) {
  return (
    <span className={cn("buzz-shimmer", className)} data-text={children}>
      {children}
    </span>
  );
}
