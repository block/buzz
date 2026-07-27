use super::super::normalize_runtime_avatars;
use super::record_with_key;

#[test]
fn loaded_devin_records_replace_only_the_superseded_default_avatar() {
    let mut devin = record_with_key("");
    devin.runtime = Some("devin".to_string());
    devin.avatar_url = Some(
        "https://mintcdn.com/cognitionai/Hhrl_8XUBqA4VQ6v/logo/favicon.svg?fit=max&auto=format&n=Hhrl_8XUBqA4VQ6v&q=85&s=ab641f30c01bf5374b90b62209db569e"
            .to_string(),
    );

    let mut custom = record_with_key("");
    custom.runtime = Some("devin".to_string());
    custom.avatar_url = Some("https://example.test/custom.png".to_string());

    normalize_runtime_avatars(std::slice::from_mut(&mut devin));
    normalize_runtime_avatars(std::slice::from_mut(&mut custom));

    assert_eq!(
        devin.avatar_url,
        crate::managed_agents::managed_agent_avatar_url("devin")
    );
    assert_eq!(
        custom.avatar_url.as_deref(),
        Some("https://example.test/custom.png")
    );
}
