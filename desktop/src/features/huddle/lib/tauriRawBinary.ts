/**
 * Raw binary invoke — uses Tauri's internal IPC for zero-copy ArrayBuffer transfer.
 *
 * The typed @tauri-apps/api doesn't expose InvokeBody::Raw. Keep the internal
 * dependency isolated here so both audio and screen-share fast paths share the
 * same escape hatch.
 */
export function invokeRawBinary(
  cmd: string,
  payload: Uint8Array,
): Promise<unknown> {
  // biome-ignore lint/suspicious/noExplicitAny: Tauri internals have no public type definition
  const internals = (window as any).__TAURI_INTERNALS__;
  if (!internals?.invoke) {
    return Promise.reject(new Error("Tauri internals not available"));
  }
  return internals.invoke(cmd, payload);
}
