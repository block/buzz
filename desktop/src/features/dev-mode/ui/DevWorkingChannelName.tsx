import "./DevWorkingChannelName.css";

/**
 * Sweeps a quiet foreground highlight across a working channel name without
 * changing any glyph's position or shape. Idle names stay as plain text.
 */
export function DevWorkingChannelName({
  name,
  working,
}: {
  name: string;
  working: boolean;
}) {
  if (!working) {
    return name;
  }

  return (
    <span
      className="dev-working-channel-name"
      data-channel-name={name}
      data-testid="dev-mode-working-channel-name"
    >
      {name}
    </span>
  );
}
