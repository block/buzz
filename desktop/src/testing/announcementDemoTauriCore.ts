import {
  invoke as nativeInvoke,
  type InvokeArgs,
  type InvokeOptions,
} from "../../node_modules/@tauri-apps/api/core.js";

export type {
  InvokeArgs,
  InvokeOptions,
} from "../../node_modules/@tauri-apps/api/core.js";
export {
  addPluginListener,
  Channel,
  checkPermissions,
  convertFileSrc,
  isTauri,
  PluginListener,
  requestPermissions,
  Resource,
  SERIALIZE_TO_IPC_FN,
  transformCallback,
} from "../../node_modules/@tauri-apps/api/core.js";

type AnnouncementDemoWindow = Window & {
  __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
    command: string,
    payload?: unknown,
  ) => unknown;
};

/** Route native-shell commands through the deterministic announcement bridge. */
export function invoke<T>(
  command: string,
  payload?: InvokeArgs,
  options?: InvokeOptions,
): Promise<T> {
  const mockInvoke = (window as AnnouncementDemoWindow)
    .__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
  if (mockInvoke) {
    return Promise.resolve(mockInvoke(command, payload)) as Promise<T>;
  }
  return nativeInvoke<T>(command, payload, options);
}
