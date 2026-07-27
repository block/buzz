import * as React from "react";

import {
  getModelRoutingPreference,
  setModelRoutingPreference,
  type ModelRoutingPreference,
} from "@/shared/api/tauriCommandBrief";

export function useModelRoutingPreference() {
  const [preference, setPreferenceState] =
    React.useState<ModelRoutingPreference>("local_first");
  const [loading, setLoading] = React.useState(true);
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let active = true;
    void getModelRoutingPreference()
      .then((value) => {
        if (active) {
          setPreferenceState(value);
          setError(null);
        }
      })
      .catch(() => {
        if (active) setError("Model routing preference is unavailable.");
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, []);

  const setPreference = React.useCallback(
    async (next: ModelRoutingPreference) => {
      if (saving || next === preference) return;
      const previous = preference;
      setPreferenceState(next);
      setSaving(true);
      setError(null);
      try {
        setPreferenceState(await setModelRoutingPreference(next));
      } catch {
        setPreferenceState(previous);
        setError("Model routing preference could not be saved.");
      } finally {
        setSaving(false);
      }
    },
    [preference, saving],
  );

  return {
    preference,
    loading,
    saving,
    error,
    setPreference,
  };
}
