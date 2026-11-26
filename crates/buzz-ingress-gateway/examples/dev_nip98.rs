//! Development helper for running the gateway locally.
//!
//! ```text
//! cargo run -q -p buzz-ingress-gateway --example dev_nip98 -- keygen
//! cargo run -q -p buzz-ingress-gateway --example dev_nip98 -- sign <secret-key> <METHOD> <url> [body-file]
//! ```
//!
//! `keygen` prints a new secret key and its public key, both hex. `sign`
//! prints an `Authorization` header value: a NIP-98 event for the method and
//! URL, with a `payload` tag hashing the body file when one is given. The
//! management API accepts it when the key's public key is listed in
//! `BUZZ_INGRESS_RELAY_PUBKEYS`.
use nostr::hashes::{sha256::Hash as Sha256Hash, Hash};
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::{Keys, Url};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owned: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = owned.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["keygen"] => {
            let keys = Keys::generate();
            println!("secret_key={}", keys.secret_key().to_secret_hex());
            println!("public_key={}", keys.public_key().to_hex());
        }
        ["sign", key, method, url, rest @ ..] if rest.len() <= 1 => {
            let keys = Keys::parse(key)?;
            let method = match *method {
                "GET" => HttpMethod::GET,
                "POST" => HttpMethod::POST,
                "PUT" => HttpMethod::PUT,
                "PATCH" => HttpMethod::PATCH,
                other => return Err(format!("unsupported method {other}").into()),
            };
            let mut request = HttpData::new(Url::parse(url)?, method);
            if let Some(path) = rest.first() {
                request = request.payload(Sha256Hash::hash(&std::fs::read(path)?));
            }
            println!("{}", request.to_authorization(&keys).await?);
        }
        _ => {
            return Err(
                "usage: dev_nip98 keygen | dev_nip98 sign <secret-key> <METHOD> <url> [body-file]"
                    .into(),
            )
        }
    }
    Ok(())
}
