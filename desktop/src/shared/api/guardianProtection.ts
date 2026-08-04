export type GuardianProtectionLevel = "l0" | "l1" | "l2" | "l3";

export type RawGuardianProtection = {
  level: GuardianProtectionLevel;
  summary: string;
  lockdown_allowed: boolean;
};

export type GuardianProtection = {
  level: GuardianProtectionLevel;
  summary: string;
  lockdownAllowed: boolean;
};

export function fromRawGuardianProtection(
  raw?: RawGuardianProtection,
): GuardianProtection {
  return raw
    ? {
        level: raw.level,
        summary: raw.summary,
        lockdownAllowed: raw.lockdown_allowed,
      }
    : {
        level: "l0",
        summary: "Protection status unavailable",
        lockdownAllowed: false,
      };
}
