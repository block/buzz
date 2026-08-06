export function CurrentRelayBindingBadge() {
  return (
    <span
      aria-label="Current relay binding"
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
        <path
          d="M9.5 14.5 7 12l-1.5 1.5 4 4 9-9L17 7l-7.5 7.5Z"
          fill="currentColor"
        />
      </svg>
    </span>
  );
}
