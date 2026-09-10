use super::*;

#[test]
fn channel_messages_require_text_or_media() {
    let channel_id = Uuid::new_v4();
    assert!(build_message(
        channel_id,
        " \n\t",
        None,
        &[],
        &[],
        &[],
        &[],
        &[],
        None,
        "https://relay.example",
    )
    .is_err());

    let media = vec![vec![
        "imeta".to_string(),
        "url https://cdn.example/image.png".to_string(),
    ]];
    assert!(build_message(
        channel_id,
        "",
        None,
        &[],
        &media,
        &[],
        &[],
        &[],
        None,
        "https://relay.example",
    )
    .is_ok());
}
