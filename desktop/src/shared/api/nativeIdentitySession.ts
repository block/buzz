/** Public, memory-only identity of this renderer realm. Never stores credentials.
 * A remote realm is bound once; replacement requires a webview reload so stale
 * callbacks cannot resolve a replacement generation from a mutable singleton.
 */
export type NativeIdentityStatus = {
  mode: "local" | "remote";
  authState: "local" | "authenticated" | "logging-in" | "signed-out";
  publicIdentity: string | null;
  generation: number;
  workspaceActive: boolean;
};
let realm: NativeIdentityStatus | null = null;
let revoked = false;
export function bindNativeIdentity(status: NativeIdentityStatus): void {
  if (realm?.mode === "remote" && realm.generation !== status.generation) {
    throw new Error("A new native identity requires a fresh renderer realm");
  }
  realm = status;
}
export function isRemoteIdentity(): boolean {
  return realm?.mode === "remote";
}
export function nativeIdentity(): NativeIdentityStatus | null {
  return realm;
}
export function revokeNativeIdentity(): void {
  revoked = true;
}
export function nativeGeneration(): number | undefined {
  if (realm?.mode !== "remote") return undefined;
  if (revoked || realm.authState !== "authenticated")
    throw new Error("Sign in to Buzz again");
  return realm.generation;
}
