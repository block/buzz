use super::*;
fn reference() -> RuntimeConfigurationRef {
    RuntimeConfigurationRef {
        id: uuid::Uuid::new_v4().to_string(),
        revision: uuid::Uuid::new_v4().to_string(),
    }
}
fn request(action: Action) -> Request {
    Request {
        target: StopTarget {
            v: 1,
            community: "wss://one.example".into(),
            desktop: "a".repeat(32),
            agent: Keys::generate().public_key().to_hex(),
        },
        action,
        observed: None,
        configuration: (action == Action::Preflight).then(reference),
        cursor: None,
    }
}
#[test]
fn encrypted_catalog_scope_cursor_expiry_and_payload_bound() {
    let keys = Keys::generate();
    let request = request(Action::Catalog);
    let event = request.sign(&keys).unwrap();
    let entry = RuntimeConfigurationSummary {
        configuration: reference(),
        name: "n".repeat(120),
        host: request.target.desktop.clone(),
        runtime: "r".repeat(128),
        model: "m".repeat(512),
        provider: Some("p".repeat(128)),
        eligible: true,
    };
    let result = ResultMessage {
        request: request.clone(),
        id: event.id.to_hex(),
        outcome: Outcome::Ready,
        observation: Some(Observation {
            valid_until: event.created_at.as_secs() + 30,
            running_configuration: None,
            catalog: Some(CatalogPage {
                next: Some(entry.configuration.id.clone()),
                entry: Some(entry),
            }),
        }),
    };
    let signed = result.sign(&keys).unwrap();
    assert!(signed.content.len() <= 4096);
    assert_eq!(
        ResultMessage::read(&signed, &keys, &event, &request.target.community).unwrap(),
        result
    );
    assert!(ResultMessage::read(
        &signed,
        &Keys::generate(),
        &event,
        &request.target.community
    )
    .is_err());
    assert!(ResultMessage::read(&signed, &keys, &event, "wss://other.example").is_err());
    for mutation in 0..6 {
        let mut invalid = result.clone();
        let observation = invalid.observation.as_mut().unwrap();
        match mutation {
            0 => observation.valid_until += 1,
            1 => observation.valid_until = event.created_at.as_secs(),
            2 => {
                observation
                    .catalog
                    .as_mut()
                    .unwrap()
                    .entry
                    .as_mut()
                    .unwrap()
                    .host = "b".repeat(32)
            }
            3 => observation.catalog.as_mut().unwrap().next = Some(reference().id),
            4 => observation.catalog = None,
            _ => {
                invalid.request.cursor = invalid
                    .observation
                    .as_ref()
                    .unwrap()
                    .catalog
                    .as_ref()
                    .unwrap()
                    .next
                    .clone()
            }
        }
        assert!(invalid
            .validate_observation(event.created_at.as_secs())
            .is_err());
    }
}
#[test]
fn exact_ref_is_bound_and_unknown_running_never_satisfies_start() {
    let keys = Keys::generate();
    let mut request = request(Action::Preflight);
    let event = request.sign(&keys).unwrap();
    let mut result = ResultMessage {
        request: request.clone(),
        id: event.id.to_hex(),
        outcome: Outcome::Ready,
        observation: Some(Observation {
            valid_until: event.created_at.as_secs() + 30,
            running_configuration: None,
            catalog: None,
        }),
    };
    result.request.configuration = Some(reference());
    assert!(ResultMessage::read(
        &result.sign(&keys).unwrap(),
        &keys,
        &event,
        &request.target.community
    )
    .is_err());
    request.action = Action::Start;
    result.request = request;
    result.outcome = Outcome::Running;
    assert!(result
        .validate_observation(event.created_at.as_secs())
        .is_err());
    result.observation.as_mut().unwrap().running_configuration =
        result.request.configuration.clone();
    assert!(result
        .validate_observation(event.created_at.as_secs())
        .is_ok());
    result.request.cursor = Some(reference().id);
    assert!(result.request.sign(&keys).is_err());
    result.request.cursor = None;
    result.request.configuration.as_mut().unwrap().revision = "latest".into();
    assert!(result.request.sign(&keys).is_err());
}
