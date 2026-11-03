//! Preserve pairing events delivered before the subscription's EOSE marker.
use std::time::Duration;

use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::{Error, Message};

pub(super) async fn wait_for_eose<S>(
    read: &mut S,
    sub_id: &str,
    duration: Duration,
) -> Result<Vec<Message>, String>
where
    S: StreamExt<Item = Result<Message, Error>> + Unpin,
{
    tokio::time::timeout(duration, async {
        let mut pending = Vec::new();
        loop {
            let message = read
                .next()
                .await
                .ok_or_else(|| "relay closed waiting for EOSE".to_string())?
                .map_err(|error| format!("WS error waiting for EOSE: {error}"))?;
            let Message::Text(text) = &message else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(text.as_str()) else {
                continue;
            };
            let Some(values) = value.as_array() else {
                continue;
            };
            if values.get(1).and_then(|value| value.as_str()) != Some(sub_id) {
                continue;
            }
            match values.first().and_then(|value| value.as_str()) {
                Some("EOSE") => return Ok(pending),
                Some("CLOSED") => {
                    return Err("The relay closed the pairing subscription. Try again.".into())
                }
                Some("EVENT") if values.len() >= 3 => {
                    // A session needs only one offer; bound untrusted setup traffic.
                    if pending.len() >= 32 {
                        return Err("Too many pairing events during connection setup".into());
                    }
                    pending.push(message);
                }
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| "timeout waiting for EOSE".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    fn text(value: &str) -> Result<Message, Error> {
        Ok(Message::Text(value.into()))
    }

    #[tokio::test]
    async fn early_scan_is_preserved_in_order_before_live_messages() {
        let offer = r#"["EVENT","pair",{"content":"encrypted offer"}]"#;
        let confirmation = r#"["EVENT","pair",{"content":"encrypted confirmation"}]"#;
        let mut read = stream::iter(vec![
            text(offer),
            text(r#"["EOSE","other"]"#),
            text(r#"["EOSE","pair"]"#),
            text(confirmation),
        ]);
        let buffered = wait_for_eose(&mut read, "pair", Duration::from_secs(1))
            .await
            .expect("ready");
        assert_eq!(buffered, vec![Message::Text(offer.into())]);
        let mut combined = stream::iter(buffered.into_iter().map(Ok)).chain(read);
        assert_eq!(
            combined.next().await.expect("offer").expect("message"),
            Message::Text(offer.into())
        );
        assert_eq!(
            combined
                .next()
                .await
                .expect("confirmation")
                .expect("message"),
            Message::Text(confirmation.into())
        );
    }

    #[tokio::test]
    async fn closed_subscription_fails_instead_of_showing_a_dead_qr() {
        let mut read = stream::iter(vec![text(r#"["CLOSED","pair","not allowed"]"#)]);
        assert!(wait_for_eose(&mut read, "pair", Duration::from_secs(1))
            .await
            .expect_err("closed")
            .contains("closed"));
    }

    #[tokio::test]
    async fn missing_eose_times_out() {
        let mut read = stream::pending::<Result<Message, Error>>();
        assert!(wait_for_eose(&mut read, "pair", Duration::from_millis(1))
            .await
            .expect_err("timeout")
            .contains("timeout"));
    }

    #[tokio::test]
    async fn event_buffer_is_bounded() {
        let mut read = stream::iter((0..33).map(|_| text(r#"["EVENT","pair",{}]"#)));
        assert!(wait_for_eose(&mut read, "pair", Duration::from_secs(1))
            .await
            .expect_err("bound")
            .contains("Too many"));
    }
}
