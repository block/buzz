export const MODEL_OVERRIDE_ENV_KEY = "BUZZ_DESKTOP_MODEL_OVERRIDE";
export const PROVIDER_OVERRIDE_ENV_KEY = "BUZZ_DESKTOP_PROVIDER_OVERRIDE";

export function applyLinkedModelProviderOverrides({
  envVars,
  linked,
  model,
  modelTouched,
  provider,
  providerTouched,
}: {
  envVars: Record<string, string>;
  linked: boolean;
  model: string | null;
  modelTouched: boolean;
  provider: string | null;
  providerTouched: boolean;
}): Record<string, string> {
  if (!linked || (!modelTouched && !providerTouched)) return envVars;

  const next = { ...envVars };
  if (modelTouched) {
    const value = model?.trim() ?? "";
    if (value) next[MODEL_OVERRIDE_ENV_KEY] = value;
    else delete next[MODEL_OVERRIDE_ENV_KEY];
  }
  if (providerTouched) {
    const value = provider?.trim() ?? "";
    if (value) next[PROVIDER_OVERRIDE_ENV_KEY] = value;
    else delete next[PROVIDER_OVERRIDE_ENV_KEY];
  }
  return next;
}
