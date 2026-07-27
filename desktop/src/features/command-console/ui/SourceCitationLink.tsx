import type { MouseEvent } from "react";

function revealSource(sourceId: string): void {
  const target = document.getElementById(`command-brief-source-${sourceId}`);
  if (!(target instanceof HTMLElement)) {
    return;
  }
  const disclosure = target.closest("details");
  if (disclosure instanceof HTMLDetailsElement) {
    disclosure.open = true;
  }
  target.scrollIntoView({ block: "center" });
  target.focus({ preventScroll: true });
}

export function SourceCitationLink({ sourceId }: { sourceId: string }) {
  const handleClick = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    revealSource(sourceId);
  };

  return (
    <a
      className="font-medium text-primary underline underline-offset-2"
      href={`#command-brief-source-${sourceId}`}
      onClick={handleClick}
    >
      [{sourceId}]
    </a>
  );
}
