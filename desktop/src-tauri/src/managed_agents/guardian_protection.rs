use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
// L1/L3 are wire-contract states before an adapter earns the qualification.
#[allow(dead_code)]
pub enum GuardianProtectionLevel {
    L0,
    L1,
    L2,
    L3,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GuardianRuntimeProtection {
    pub level: GuardianProtectionLevel,
    pub summary: String,
    pub lockdown_allowed: bool,
}
