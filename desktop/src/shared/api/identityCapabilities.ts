/** Build capability set by the native login gate before any community UI mounts.
 * Not identity/community data: remains fixed across login and community remounts.
 * Native commands independently enforce these exclusions (this is UI policy only).
 */
let managedIdentity = false;

export function setManagedIdentityMode(enabled: boolean): void {
  managedIdentity = enabled;
}

export function supportsLocalIdentityFeatures(): boolean {
  return !managedIdentity;
}

export function requireLocalIdentityFeature(feature: string): void {
  if (managedIdentity) {
    throw new Error(
      `${feature} is unavailable with an organization-managed identity.`,
    );
  }
}
