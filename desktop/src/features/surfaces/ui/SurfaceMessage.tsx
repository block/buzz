import { parseSurfaceSpec } from "@/features/surfaces/spec";
import SurfaceCard from "@/features/surfaces/ui/SurfaceCard";

// Timeline entry point for a surface event: parse tolerantly and render per
// the v1 fallback matrix. Every failure path is plain escaped text — never
// the markdown pipeline (markdown would reopen link/media behavior the
// data-only model closes), never a blank or error row.
export default function SurfaceMessage({ content }: { content: string }) {
  const parsed = parseSurfaceSpec(content);

  switch (parsed.outcome) {
    case "card":
      return <SurfaceCard spec={parsed.spec} />;
    case "fallback":
      return (
        <p
          className="whitespace-pre-wrap break-words text-sm text-foreground/90"
          data-testid="surface-fallback"
        >
          {parsed.text}
        </p>
      );
    case "raw":
      return (
        <p
          className="whitespace-pre-wrap break-words text-sm text-muted-foreground"
          data-testid="surface-raw"
        >
          {content}
        </p>
      );
  }
}
