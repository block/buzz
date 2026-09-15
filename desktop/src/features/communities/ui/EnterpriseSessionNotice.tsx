import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { Button } from "@/shared/ui/button";
import { relayClient } from "@/shared/api/relayClient";

/** Keep drafts mounted when enterprise authority expires; offer recovery in place. */
export function EnterpriseSessionNotice() {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    async function check() {
      try {
        if (await invoke<boolean>("federated_identity_required")) {
          await invoke("acquire_federated_assertion");
        }
        if (active) setError(null);
      } catch (error) {
        if (active) setError(String(error));
      } finally {
        if (active) timer = setTimeout(check, 10_000);
      }
    }
    void check();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, []);
  if (!error) return null;
  return (
    <div
      role="alert"
      className="fixed bottom-4 left-1/2 z-50 flex max-w-lg -translate-x-1/2 items-center gap-3 rounded-lg border bg-background p-4 shadow-lg"
    >
      <p className="text-sm">
        Work-account connection paused. Your drafts are still here. {error}
      </p>
      <Button
        disabled={busy}
        type="button"
        onClick={async () => {
          setBusy(true);
          try {
            await invoke("start_builderlab_login");
            await relayClient.preconnect();
            setError(null);
          } catch (error) {
            setError(String(error));
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? "Signing in…" : "Sign in"}
      </Button>
    </div>
  );
}
