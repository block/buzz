/** Compact device-local audience indicator above the message editor. */
export function ComposerAudienceHint({ hint }: { hint: string | null }) {
  if (!hint) return null;
  return (
    <p
      className="mb-2 text-2xs text-muted-foreground"
      data-testid="composer-agent-audience-hint"
    >
      {hint}
    </p>
  );
}
