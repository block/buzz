use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::validate_repo_id;

pub async fn cmd_create_repo(
    client: &BuzzClient,
    repo_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    clone_urls: &[String],
    web_url: Option<&str>,
    relays: &[String],
) -> Result<(), CliError> {
    validate_repo_id(repo_id)?;

    let clone_refs: Vec<&str> = clone_urls.iter().map(|s| s.as_str()).collect();
    let relay_refs: Vec<&str> = relays.iter().map(|s| s.as_str()).collect();

    let builder = buzz_sdk::build_repo_announcement(
        repo_id,
        name,
        description,
        &clone_refs,
        web_url,
        &relay_refs,
    )
    .map_err(|e| CliError::Other(format!("build_repo_announcement failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_get_repo(
    client: &BuzzClient,
    repo_id: &str,
    owner: Option<&str>,
) -> Result<(), CliError> {
    validate_repo_id(repo_id)?;

    let mut filter = serde_json::json!({
        "kinds": [30617],
        "#d": [repo_id]
    });

    // If owner specified, filter by author pubkey; otherwise return any match.
    // Note: without --owner, multiple repos with the same name (different owners) may be returned.
    if let Some(pk) = owner {
        crate::validate::validate_hex64(pk)?;
        filter["authors"] = serde_json::json!([pk]);
    }

    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_list_repos(
    client: &BuzzClient,
    owner: Option<&str>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    // Default to self if no owner specified.
    let pubkey = match owner {
        Some(pk) => {
            crate::validate::validate_hex64(pk)?;
            pk.to_string()
        }
        None => client.keys().public_key().to_hex(),
    };

    let mut filter = serde_json::json!({
        "kinds": [30617],
        "authors": [pubkey]
    });

    if let Some(n) = limit {
        filter["limit"] = serde_json::json!(n);
    }

    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn dispatch(cmd: crate::ReposCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::ReposCmd;
    match cmd {
        ReposCmd::Create {
            id,
            name,
            description,
            clone_urls,
            web,
            relays,
        } => {
            cmd_create_repo(
                client,
                &id,
                name.as_deref(),
                description.as_deref(),
                &clone_urls,
                web.as_deref(),
                &relays,
            )
            .await
        }
        ReposCmd::Get { id, owner } => cmd_get_repo(client, &id, owner.as_deref()).await,
        ReposCmd::List { owner, limit } => cmd_list_repos(client, owner.as_deref(), limit).await,
    }
}
