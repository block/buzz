//! Print the hex public key for a secret key (hex or nsec). Dev utility.
fn main() -> anyhow::Result<()> {
    let secret = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: pubkey_of <secret>"))?;
    let keys = nostr::Keys::parse(&secret)?;
    println!("{}", keys.public_key().to_hex());
    Ok(())
}
