use crate::managed_agents::AgentEffortOption;

pub(super) fn normalize_effort_options(
    raw: &serde_json::Value,
) -> (Vec<AgentEffortOption>, Option<String>) {
    let effort_config = raw["stable"]["configOptions"]
        .as_array()
        .and_then(|options| {
            options.iter().find(|option| {
                option.get("category").and_then(|value| value.as_str()) == Some("thought_level")
            })
        });
    let options = effort_config
        .and_then(|option| option.get("options"))
        .and_then(|options| options.as_array())
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    let value = option.get("value")?.as_str()?.to_string();
                    let label = option
                        .get("name")
                        .or_else(|| option.get("displayName"))
                        .and_then(|name| name.as_str())
                        .unwrap_or(&value)
                        .to_string();
                    Some(AgentEffortOption { value, label })
                })
                .collect()
        })
        .unwrap_or_default();
    let current_value = effort_config
        .and_then(|option| option.get("currentValue"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    (options, current_value)
}
