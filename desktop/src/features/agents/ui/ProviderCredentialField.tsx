import type { AcpProviderProfile } from "@/shared/api/types";
import { getProviderApiKeyLabel } from "./agentConfigOptions";
import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField";
import {
  providerSecretDeviceField,
  type ProviderSecretState,
} from "./providerSecretCredentialState";

/** Catalog-aware credential field shared by Defaults, Create, and Edit. */
export function ProviderCredentialField({
  disabled,
  envVarName,
  inheritedLabel,
  isInherited,
  isRequired,
  onValueChange,
  provider,
  providerProfiles,
  providerSecret,
  value,
}: {
  disabled: boolean;
  envVarName?: string;
  inheritedLabel: string;
  isInherited: boolean;
  isRequired: boolean;
  onValueChange: (next: string) => void;
  provider: string;
  providerProfiles?: readonly AcpProviderProfile[];
  providerSecret: ProviderSecretState;
  value: string;
}) {
  return (
    <PersonaProviderApiKeyField
      deviceSecret={providerSecretDeviceField(providerSecret)}
      disabled={disabled}
      envVarName={envVarName}
      inheritedLabel={inheritedLabel}
      isInherited={isInherited}
      isRequired={isRequired && !providerSecret.configured}
      label={getProviderApiKeyLabel(provider, providerProfiles) ?? "API Key"}
      onValueChange={onValueChange}
      value={value}
    />
  );
}
