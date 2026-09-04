import type { DesktopCapabilities } from "../desktopCapabilities";

/** Read-only remote projection; local setup remains in Settings → Agents. */
export function DesktopCapabilityDetails({
  report,
  now,
}: {
  report?: DesktopCapabilities;
  now: number;
}) {
  return (
    <details className="text-xs">
      <summary>Capability details</summary>
      {!report ? (
        <p>No capability report received.</p>
      ) : (
        <>
          <p>
            Facts reported {new Date(report.reported * 1000).toLocaleString()}
            {report.reported > now &&
              " (Desktop clock ahead; report time uncertain)"}
            . Unchanged facts keep their original report time.
          </p>
          <ul>
            {report.runtimes.map((runtime) => (
              <li key={runtime.id}>
                {runtime.id}: {runtime.availability.replaceAll("_", " ")} ·
                external CLI{" "}
                {runtime.requires_external_cli ? "required" : "not required"} ·
                parallelism cap {runtime.max_parallelism ?? "not configured"}.
              </li>
            ))}
          </ul>
          {!report.runtimes.length && (
            <p>No built-in runtime facts reported.</p>
          )}
        </>
      )}
      <p>
        Cached installation facts only, not agent readiness or access to an
        agent’s signing key. Stable agent keys must be provisioned separately by
        you. For local setup and Check again, use Settings → Agents.
      </p>
    </details>
  );
}
