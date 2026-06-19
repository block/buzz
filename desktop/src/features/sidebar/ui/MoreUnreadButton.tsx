import { UnreadPill, unreadCountLabel } from "@/shared/ui/UnreadPill";

export function MoreUnreadButton({
  bottomClassName = "bottom-0",
  count,
  onClick,
  position,
  testId,
  topClassName = "top-0",
}: {
  bottomClassName?: string;
  count: number;
  onClick: () => void;
  position: "top" | "bottom";
  testId: string;
  topClassName?: string;
}) {
  const positionClassName = position === "top" ? topClassName : bottomClassName;

  return (
    <div
      className={`pointer-events-none absolute inset-x-0 z-10 flex justify-center py-1 ${positionClassName}`}
    >
      <UnreadPill
        direction={position === "top" ? "up" : "down"}
        label={unreadCountLabel(count)}
        onClick={onClick}
        testId={testId}
      />
    </div>
  );
}
