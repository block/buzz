import { useMutation, useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { toast } from "sonner";

import {
  deviceIdentityQueryKey,
  useDeviceIdentityQuery,
} from "@/features/agents/hooks";
import { setDeviceLabel } from "@/shared/api/tauriDeviceIdentity";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

/** Backend cap on the device label; mirrored here so the field stops typing at it. */
const DEVICE_LABEL_MAX_LENGTH = 32;

/**
 * Shows and renames this install's device label.
 *
 * The label is published on each local agent's kind:30177 event so the same
 * account's other devices can tell duplicate agent names apart, which is why
 * the subcopy spells out that it leaves the machine.
 */
export function DeviceNameSettingsCard() {
  const queryClient = useQueryClient();
  const deviceIdentity = useDeviceIdentityQuery();
  const savedLabel = deviceIdentity.data?.deviceLabel ?? "";
  const [draftLabel, setDraftLabel] = React.useState(savedLabel);
  const [editedLabel, setEditedLabel] = React.useState(false);

  // Seed the field from the query the first time it resolves, but never stomp
  // on what the user is currently typing.
  React.useEffect(() => {
    if (!editedLabel) {
      setDraftLabel(savedLabel);
    }
  }, [editedLabel, savedLabel]);

  const renameDevice = useMutation({
    mutationFn: (label: string) => setDeviceLabel(label),
    onSuccess: (identity) => {
      setEditedLabel(false);
      setDraftLabel(identity.deviceLabel);
      void queryClient.invalidateQueries({ queryKey: deviceIdentityQueryKey });
      toast.success("Device name updated");
    },
    onError: (error: unknown) => {
      toast.error(
        error instanceof Error ? error.message : "Could not rename this device",
      );
    },
  });

  const trimmedLabel = draftLabel.trim();
  const saveDisabled =
    trimmedLabel.length === 0 ||
    trimmedLabel === savedLabel ||
    renameDevice.isPending ||
    !deviceIdentity.data;

  return (
    <div className="min-w-0 space-y-3">
      <SettingsOptionGroup data-testid="device-name-card" title="This device">
        <SettingsOptionRow className="flex-col items-stretch gap-3 sm:flex-row sm:items-center">
          <div className="min-w-0">
            <label className="text-sm font-medium" htmlFor="device-name-input">
              Device name
            </label>
            <p
              className="text-sm font-normal text-muted-foreground/70"
              data-settings-subcopy
            >
              Shown next to this device's agents when you view them from your
              other devices. Published with your agents, so avoid personal
              details.
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <Input
              className="w-48"
              data-testid="device-name-input"
              disabled={!deviceIdentity.data || renameDevice.isPending}
              id="device-name-input"
              maxLength={DEVICE_LABEL_MAX_LENGTH}
              onChange={(event) => {
                setEditedLabel(true);
                setDraftLabel(event.target.value);
              }}
              value={draftLabel}
            />
            <Button
              data-testid="device-name-save"
              disabled={saveDisabled}
              onClick={() => {
                renameDevice.mutate(trimmedLabel);
              }}
              size="sm"
              type="button"
              variant="outline"
            >
              Save
            </Button>
          </div>
        </SettingsOptionRow>
      </SettingsOptionGroup>
    </div>
  );
}
