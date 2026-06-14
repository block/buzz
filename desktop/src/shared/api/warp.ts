import { invokeTauri } from "@/shared/api/tauri";

export async function connectWarpVpn(): Promise<void> {
  await invokeTauri("connect_warp_vpn");
}

export async function refreshWarpAccess(): Promise<void> {
  await invokeTauri("refresh_warp_access");
}
