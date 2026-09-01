import * as React from "react";

import { HermesProfileField } from "./HermesProfileField";

export function useHermesProfileDefinitionBinding(
  visible: boolean,
  isPending: boolean,
  open: boolean,
) {
  const [profile, setProfile] = React.useState("");
  React.useEffect(() => {
    if (!open) setProfile("");
  }, [open]);
  return {
    canSubmit: !visible || profile.trim().length > 0,
    profileForSubmit: visible ? profile : undefined,
    field: visible ? (
      <HermesProfileField
        disabled={isPending}
        onValueChange={setProfile}
        value={profile}
      />
    ) : null,
  };
}
