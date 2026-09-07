//! Validate the identity set affected by a persona display-name cascade.
use super::model::DeviceAgentPolicy;

/// Return the linked identity exempted from name-directory collision checks.
/// Call before saving the definition, using the same exact old-name match as
/// the propagation path (pool-named instances keep their individual names).
pub(crate) fn rename_identity<'a>(
    policy: &DeviceAgentPolicy,
    persona_id: &str,
    current_name: &str,
    requested_name: &str,
    records: impl IntoIterator<Item = (&'a str, &'a str, Option<&'a str>)>,
) -> Result<Option<String>, String> {
    let records: Vec<_> = records.into_iter().collect();
    let mut affected = std::collections::HashSet::new();
    let mut existing = None;
    for &(name, pubkey, definition) in &records {
        if definition != Some(persona_id) {
            continue;
        }
        policy.require_local_agent(name, Some(pubkey), definition)?;
        if name == current_name {
            existing = Some(pubkey.to_string());
            if current_name != requested_name.trim() {
                affected.insert(pubkey.to_ascii_lowercase());
            }
        }
    }
    if policy.unique_names && !affected.is_empty() {
        let collides = affected.len() > 1
            || records.iter().any(|(name, key, _)| {
                name.trim().eq_ignore_ascii_case(requested_name.trim())
                    && !affected.contains(&key.to_ascii_lowercase())
            });
        if collides {
            return Err(format!("Renaming this definition to {} would give multiple identities the same name. Rename its instances individually first.", requested_name.trim()));
        }
    }
    Ok(existing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_preserves_single_identity_pool_names_and_unrestricted_mode() {
        let policy = DeviceAgentPolicy {
            unique_names: true,
            ..Default::default()
        };
        let records = [
            ("Scout", "aBc", Some("persona")),
            ("Scout", "AbC", Some("persona")),
            ("Birch", "pool", Some("persona")),
            ("Scout", "unrelated", Some("other")),
        ];
        assert!(rename_identity(&policy, "persona", "Scout", " Notebook ", records).is_ok());
        assert!(rename_identity(&policy, "persona", "Scout", "Scout", records).is_ok());
        assert_eq!(
            rename_identity(&policy, "unlinked", "Scout", "Notebook", records).unwrap(),
            None
        );
        let duplicates = [
            ("Scout", "a", Some("persona")),
            ("Scout", "b", Some("persona")),
        ];
        assert!(rename_identity(
            &DeviceAgentPolicy::default(),
            "persona",
            "Scout",
            "Notebook",
            duplicates
        )
        .is_ok());
        assert!(rename_identity(&policy, "persona", "Scout", " scout ", duplicates).is_err());
    }

    #[test]
    fn renamed_identity_not_first_pool_identity_gets_the_directory_exemption() {
        let policy = DeviceAgentPolicy {
            unique_names: true,
            ..Default::default()
        };
        let records = [
            ("Birch", "pool", Some("persona")),
            ("Scout", "renamed", Some("persona")),
        ];
        assert_eq!(
            rename_identity(&policy, "persona", "Scout", "Notebook", records).unwrap(),
            Some("renamed".into())
        );
    }

    #[test]
    fn prospective_name_collides_with_normalized_pool_name() {
        let policy = DeviceAgentPolicy {
            unique_names: true,
            ..Default::default()
        };
        let records = [
            (" notebook ", "pool", Some("persona")),
            ("Scout", "renamed", Some("persona")),
        ];
        assert!(rename_identity(&policy, "persona", "Scout", "Notebook", records).is_err());
    }

    #[test]
    fn unique_name_rename_rejects_two_distinct_linked_identities() {
        let policy = DeviceAgentPolicy {
            unique_names: true,
            ..Default::default()
        };
        let records = [
            ("Scout", "key-a", Some("persona")),
            ("Scout", "key-b", Some("persona")),
        ];
        let result = rename_identity(&policy, "persona", "Scout", " Notebook ", records);
        assert!(
            result.is_err(),
            "a persona rename must not assign Notebook to two pubkeys: {result:?}"
        );
    }
}
