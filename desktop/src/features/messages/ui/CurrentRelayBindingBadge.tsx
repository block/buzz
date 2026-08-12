/** Render the informational badge for a fresh connection-local binding. */
export function CurrentRelayBindingBadge() {
  return (
    <span
      aria-label="Relay-reported current key (informational)"
      className="inline-flex shrink-0 items-center text-blue-500"
      data-testid="current-relay-binding"
      role="img"
    >
      <svg
        aria-hidden="true"
        className="h-4 w-4"
        fill="none"
        viewBox="0 0 24 24"
      >
        <circle cx="12" cy="12" fill="currentColor" r="4" />
      </svg>
    </span>
  );
}
