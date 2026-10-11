import { BUZZ_TERM_UNAVAILABLE } from "@/features/bridge/ui/WebRemoteSettings";

/** Page opened by the greyed-out Buzz term nav row on web: explains what is missing and the exact settings to turn on. */
export function RemoteUnavailablePage({ feature }: { feature: "term" }) {
  const c = BUZZ_TERM_UNAVAILABLE;
  return (
    <div className="mx-auto flex max-w-xl flex-col gap-3 p-6" data-testid="remote-unavailable-page" data-feature={feature}>
      <h1 className="text-xl font-semibold" data-testid="remote-unavailable-title">{c.title}</h1>
      <p className="text-sm text-muted-foreground">{c.summary}</p>
      <ol className="list-decimal space-y-1 pl-5 text-sm" data-testid="remote-unavailable-steps">
        {c.steps.map((t) => (
          <li key={t}>{t}</li>
        ))}
      </ol>
    </div>
  );
}
