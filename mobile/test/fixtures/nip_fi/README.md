# Community authentication discovery fixtures

These are complete `/info` response bodies emitted by the production HTTP router
at [BUZZ-511 commit d3ae4935b11e207f0736833cd5220f207be359fd](https://github.com/block/buzz/commit/d3ae4935b11e207f0736833cd5220f207be359fd).
The normative field is documented in that commit's
[NIP-FI Discovery section](https://github.com/block/buzz/blob/d3ae4935b11e207f0736833cd5220f207be359fd/docs/nips/NIP-FI.md#discovery).

| Fixture suffix | Global mode | Synthetic Host | Required | Protocol compatible |
| --- | --- | --- | --- | --- |
| `required` | off | `required.example` | true | true |
| `optional` | off | `free.example` | false | false |
| `optional_capable` | shadow | `free.example` | false | true |

The export reused `nip_fi_policy::tests::fixture` from that commit and called
`router::build_router` through `tower::ServiceExt::oneshot` for both `/` and
`/info`, with `Host` and `Accept: application/nostr+json`. It asserted HTTP 200,
`Cache-Control: no-store`, and equal decoded documents for the two routes, then
saved the unmodified `/info` body. Policy and verifier inputs are synthetic test
boundaries; discovery serialization and routing are the production implementation.
This is a wire fixture, not deployed-host or device acceptance evidence.

To regenerate, use a detached checkout of the source commit and append this
temporary test to `crates/buzz-relay/src/nip_fi_policy/tests.rs`:

```rust
#[tokio::test]
async fn export_buzz_517_discovery_fixtures() {
    let out = std::path::PathBuf::from(std::env::var("BUZZ_FIXTURE_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    for (name, mode, host) in [
        ("required", NipFiMode::Off, "required.example"),
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

From that checkout, run with an absolute output directory:

```sh
BUZZ_FIXTURE_OUT=/tmp/buzz-517-discovery cargo test -p buzz-relay --lib nip_fi_policy::tests::export_buzz_517_discovery_fixtures -- --exact
```

Copy the three output JSON bodies here with the source-commit filename prefix.
The temporary exporter does not modify the relay implementation.

Legacy relays lacking `requires_nip_fi` remain covered separately by the parser's
absent-field cases. Capability alone must never turn an optional host into a
required one. These fixtures do not configure or authorize any sign-in origin.
