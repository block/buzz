import { invoke, isTauri } from "@tauri-apps/api/core";

export function performDefaultHaptic() {
  if (!isTauri()) {
    return;
  }

  void invoke("perform_sidebar_default_haptic").catch(() => {});
}

export function performSidebarDefaultHaptic() {
  performDefaultHaptic();
}
