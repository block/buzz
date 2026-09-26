import { invoke } from "@tauri-apps/api/core";

export type EnterpriseProfileProjection = {
  username: string;
  displayName: string;
};

export type EnterpriseAuth = {
  email?: string | null;
  expiresAt: string;
  profileProjection?: EnterpriseProfileProjection | null;
};

export type EnterpriseAuthLoginAttempt = {
  attemptId: string;
};

export function getEnterpriseAuth() {
  return invoke<EnterpriseAuth | null>("get_enterprise_auth");
}

export function startEnterpriseAuthLogin(options?: EnterpriseAuthLoginAttempt) {
  return invoke<EnterpriseAuth>("start_enterprise_auth_login", options);
}

export function cancelEnterpriseAuthLogin(
  options?: EnterpriseAuthLoginAttempt,
) {
  return invoke<void>("cancel_enterprise_auth_login", options);
}

export function clearEnterpriseAuth() {
  return invoke<void>("clear_enterprise_auth");
}
