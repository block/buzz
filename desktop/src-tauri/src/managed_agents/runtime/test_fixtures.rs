pub(super) const EXPECTED_ACCESS_ENV: &str = "BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY";

pub(super) fn expected_owner_only() -> bool {
    match std::env::var(EXPECTED_ACCESS_ENV) {
        Ok(value) => value
            .parse::<bool>()
            .unwrap_or_else(|_| panic!("{EXPECTED_ACCESS_ENV} must be true or false")),
        Err(std::env::VarError::NotPresent)
            if !crate::managed_agents::owner_only_access_build() =>
        {
            false
        }
        Err(std::env::VarError::NotPresent) => {
            panic!("{EXPECTED_ACCESS_ENV} must be set for owner-only-access-build tests")
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("{EXPECTED_ACCESS_ENV} must be valid UTF-8")
        }
    }
}

pub(super) fn expected_mode(oss_mode: &'static str) -> &'static str {
    if expected_owner_only() {
        "owner-only"
    } else {
        oss_mode
    }
}
