# Community authentication discovery fixtures

These complete HTTP response bodies come from the production router at
[main commit 6b8a72e8355125d24e4f4fab1e083dced4119f5c](https://github.com/block/buzz/commit/6b8a72e8355125d24e4f4fab1e083dced4119f5c).
The [NIP-FI discovery contract](https://github.com/block/buzz/blob/6b8a72e8355125d24e4f4fab1e083dced4119f5c/docs/nips/NIP-FI.md#discovery)
defines the top-level `requires_nip_fi` boolean.

| Fixture suffix | Global mode | Synthetic Host | Required | FI capability |
| --- | --- | --- | --- | --- |
| `required` | shadow | `required.example` | true | true |
| `optional` | off | `free.example` | false | false |
| `optional_capable` | shadow | `free.example` | false | true |

The required fixture uses Shadow mode: Off never requires federated identity,
even when the stored community policy requires it. Capability alone does not
make an optional host required.

The exporter calls `router::build_router` through `tower::ServiceExt::oneshot`
with synthetic policy and verifier boundaries. It verifies HTTP 200,
`Cache-Control: no-store`, and matching decoded documents from `/` and `/info`,
then saves the unmodified `/info` body. These fixtures verify wire compatibility;
they do not establish deployed-host or installed-app acceptance.

To reproduce, use a detached checkout of the pinned source commit and append
this temporary exporter to `crates/buzz-relay/src/nip_fi_policy/tests.rs`:

```rust
#[tokio::test]
async fn export_buzz_517_discovery_fixtures() {
    let out = std::path::PathBuf::from(std::env::var("BUZZ_FIXTURE_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    for (name, mode, host) in [
        ("required", NipFiMode::Shadow, "required.example"),
        ("optional", NipFiMode::Off, "free.example"),
        ("optional_capable", NipFiMode::Shadow, "free.example"),
    ] {
        let (state, _, _, _) = fixture(mode).await;
        let app = crate::router::build_router(state);
        let mut documents = Vec::new();
        for path in ["/", "/info"] {
            let response = app.clone().oneshot(
                Request::get(path)
                    .header(header::HOST, host)
                    .header(header::ACCEPT, "application/nostr+json")
                    .body(Body::empty()).unwrap()
            ).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let bytes = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
            documents.push(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap());
            if path == "/info" {
                std::fs::write(out.join(format!("{name}.json")), &bytes).unwrap();
            }
        }
        assert_eq!(documents[0], documents[1]);
    }
}
```

Run from that checkout with an absolute output directory:

```sh
BUZZ_FIXTURE_OUT=/tmp/buzz-517-discovery cargo test --locked -p buzz-relay --lib nip_fi_policy::tests::export_buzz_517_discovery_fixtures -- --exact
```

The files here retain those response bytes with the `buzz_main_6b8a72e835_`
filename prefix. Legacy relays without the field are covered by the native
reader's absent-field cases.
