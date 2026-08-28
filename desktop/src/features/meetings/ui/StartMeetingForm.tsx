import * as React from "react";

import { validateMeetingRoomName } from "@/features/meetings/ui/meetingRoomName";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

type StartMeetingFormProps = {
  initialValue?: string;
  autoFocus?: boolean;
  isPending: boolean;
  /** Render the "hosting not enabled" panel instead of the transient error. */
  hostingError: boolean;
  /** Transient (retryable) error message, if the last attempt failed. */
  errorMessage?: string;
  onSubmit: (roomName: string) => void;
  onSetupHosting: () => void;
};

export function StartMeetingForm({
  initialValue = "",
  autoFocus = false,
  isPending,
  hostingError,
  errorMessage,
  onSubmit,
  onSetupHosting,
}: StartMeetingFormProps) {
  const [value, setValue] = React.useState(initialValue);
  const [localError, setLocalError] = React.useState<string | null>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (autoFocus) inputRef.current?.focus();
  }, [autoFocus]);

  const handleSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    const result = validateMeetingRoomName(value);
    if (!result.ok) {
      setLocalError(result.reason);
      return;
    }
    setLocalError(null);
    onSubmit(result.value);
  };

  return (
    <form
      className="space-y-2"
      data-testid="start-meeting-form"
      onSubmit={handleSubmit}
    >
      <div className="flex items-center gap-2">
        <Input
          aria-label="Room name"
          disabled={isPending}
          onChange={(event) => setValue(event.target.value)}
          placeholder="Room name"
          ref={inputRef}
          value={value}
        />
        <Button disabled={isPending || value.trim().length === 0} type="submit">
          Start meeting
        </Button>
      </div>
      {localError ? (
        <p className="text-xs text-destructive">{localError}</p>
      ) : null}
      {errorMessage && !hostingError ? (
        <p className="text-xs text-destructive">{errorMessage}</p>
      ) : null}
      {hostingError ? (
        <div
          className="space-y-2 rounded-xl border border-border/70 bg-muted/40 p-4"
          data-testid="meeting-hosting-panel"
        >
          <p className="text-sm font-medium">Hosting not enabled</p>
          <p className="text-xs text-muted-foreground">
            Starting a meeting needs an active HiveTalk subscription. Set one up
            with any Lightning wallet — you'll come straight back here.
          </p>
          <Button
            onClick={onSetupHosting}
            size="sm"
            type="button"
            variant="outline"
          >
            Set up hosting
          </Button>
        </div>
      ) : null}
    </form>
  );
}
