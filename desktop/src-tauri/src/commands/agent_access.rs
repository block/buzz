/// Return whether this build enforces owner-only managed-agent access.
#[tauri::command]
pub fn agent_access_owner_only() -> bool {
    crate::managed_agents::owner_only_access_build()
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY"]
    fn compiled_policy_matches_expected() {
        let expected = std::env::var("BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY")
            .expect("BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY must be set")
            .parse::<bool>()
            .expect("BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY must be true or false");
        assert_eq!(super::agent_access_owner_only(), expected);
    }
}
