use buzz_ifc_broker::{Broker, EnforcementMode, PROTOCOL_VERSION};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    protocol_version: String,
    request: Value,
    expected_result: Value,
}

#[test]
fn domain_golden_fixture_matches_the_wire_contract() {
    let fixture: Fixture = serde_json::from_str(include_str!("fixtures/domain-golden.json"))
        .expect("valid conformance fixture");
    assert_eq!(fixture.protocol_version, PROTOCOL_VERSION);

    let mut broker = Broker::new(EnforcementMode::Enforce);
    let response: Value = serde_json::from_str(&broker.handle_line(&fixture.request.to_string()))
        .expect("valid broker response");
    assert_eq!(response["result"], fixture.expected_result);
}
