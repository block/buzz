fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: gen_auth_tag <owner_sk> <agent_pk> [conditions]");
        std::process::exit(1);
    }
    let owner_sk = nostr::SecretKey::from_hex(&args[1]).unwrap();
    let owner_keys = nostr::Keys::new(owner_sk);
    let agent_pk = nostr::PublicKey::from_hex(&args[2]).unwrap();
    let conditions = args.get(3).map(|s| s.as_str()).unwrap_or("");
    let tag_json =
        sprout_sdk::nip_oa::compute_auth_tag(&owner_keys, &agent_pk, conditions).unwrap();
    println!("{}", tag_json);
}
