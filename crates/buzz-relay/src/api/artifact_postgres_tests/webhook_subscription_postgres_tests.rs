//! NIP-WH: `buzz.webhook-subscription` artifacts need channel admins and valid
//! CEL; other artifact types keep the NIP-AR writer rule
//! (`writes_use_channel_gates_for_home_and_move_source`).
use super::*;

const INTEGRATION: &str = "0f4c1a52-7d3e-4b8a-9c61-2e5f8d7a1b30";

impl Fixture {
    /// A subscription revision signed by `key`, with its integration tag.
    fn subscription(
        &self,
        key: &Keys,
        home: Uuid,
        id: Uuid,
        op: &str,
        prev: Option<&Event>,
        filter: &str,
    ) -> Event {
        self.subscription_with(
            key,
            home,
            id,
            op,
            prev,
            json!({"version": 1, "filter": filter}),
        )
    }

    /// A subscription revision with `content`, signed by `key`.
    pub(super) fn subscription_with(
        &self,
        key: &Keys,
        home: Uuid,
        id: Uuid,
        op: &str,
        prev: Option<&Event>,
        content: Value,
    ) -> Event {
        let mut tags = vec![
            vec!["ar".to_owned(), "1".into()],
            vec!["d".into(), id.to_string()],
            vec!["h".into(), home.to_string()],
            vec!["type".into(), "buzz.webhook-subscription".into()],
            vec!["op".into(), op.into()],
        ];
        let content = if op == "delete" {
            String::new()
        } else {
            tags.push(vec!["title".into(), "Opened pull requests".into()]);
            tags.push(vec!["integration".into(), INTEGRATION.into()]);
            content.to_string()
        };
        if let Some(prev) = prev {
            tags.push(vec!["prev".into(), prev.id.to_hex()]);
        }
        EventBuilder::new(Kind::Custom(45010), content)
            .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
            .sign_with_keys(key)
            .unwrap()
    }

    pub(super) async fn set_role(&self, channel: Uuid, key: &Keys, role: &str) {
        sqlx::query(
            "INSERT INTO channel_members(community_id,channel_id,pubkey,role) \
             VALUES($1,$2,$3,$4::member_role) \
             ON CONFLICT (community_id,channel_id,pubkey) DO UPDATE SET role=EXCLUDED.role",
        )
        .bind(self.community.as_uuid())
        .bind(channel)
        .bind(key.public_key().as_bytes().as_slice())
        .bind(role)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    /// Publish as the event's signer and return the refusal message.
    async fn refused(&self, key: &Keys, event: &Event) -> String {
        let (status, body) = self.request_as(key, "/events", json!(event)).await;
        assert!(
            !status.is_success() || body["accepted"] == false,
            "{status}: {body}"
        );
        body.to_string()
    }

    pub(super) async fn accepted(&self, key: &Keys, event: &Event) {
        let (status, body) = self.request_as(key, "/events", json!(event)).await;
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(body["accepted"], true, "{body}");
    }

    pub(super) async fn head(&self, id: Uuid) -> Option<(Vec<u8>, Uuid)> {
        sqlx::query_as(
            "SELECT event_id, channel_id FROM artifact_heads WHERE community_id=$1 AND artifact_id=$2",
        )
        .bind(self.community.as_uuid())
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn webhook_subscriptions_need_channel_admins() {
    const RESTRICTED: &str =
        "restricted: only channel admins can write buzz.webhook-subscription artifacts";
    let f = Fixture::new().await;
    let member = Keys::generate();
    f.set_role(f.home, &member, "member").await;
    f.set_role(f.private, &member, "member").await;
    let d = Uuid::new_v4();
    let open = r#"event.type == "pull_request""#;

    // Kind-9 writers who are not admins cannot create one, and nothing is stored.
    for key in [&f.peer, &member] {
        let create = f.subscription(key, f.home, d, "create", None, open);
        assert!(f.refused(key, &create).await.contains(RESTRICTED));
    }
    assert_eq!(f.head(d).await, None);

    // An admin's mistakes are refused when saved.
    let bad_cel = f.subscription(&f.owner, f.home, d, "create", None, "event.type ==");
    assert!(f
        .refused(&f.owner, &bad_cel)
        .await
        .contains("invalid: filter:"));
    let untagged = EventBuilder::new(
        Kind::Custom(45010),
        json!({"version":1,"filter":open}).to_string(),
    )
    .tags(
        bad_cel
            .tags
            .iter()
            .filter(|t| t.as_slice()[0] != "integration")
            .cloned(),
    )
    .sign_with_keys(&f.owner)
    .unwrap();
    assert!(f
        .refused(&f.owner, &untagged)
        .await
        .contains("exactly one integration tag"));
    assert_eq!(f.head(d).await, None);

    let create = f.subscription(&f.owner, f.home, d, "create", None, open);
    f.accepted(&f.owner, &create).await;

    // Edits need an admin too, and a promotion takes effect on the next write.
    let edit = f.subscription(&member, f.home, d, "update", Some(&create), "true");
    assert!(f.refused(&member, &edit).await.contains(RESTRICTED));
    f.set_role(f.home, &member, "admin").await;
    f.accepted(&member, &edit).await;

    // A move needs an admin in both channels: first the destination, then the source.
    let to_private = f.subscription(&member, f.private, d, "move", Some(&edit), "true");
    assert!(f.refused(&member, &to_private).await.contains(RESTRICTED));
    f.set_role(f.private, &member, "admin").await;
    f.set_role(f.home, &member, "member").await;
    assert!(f.refused(&member, &to_private).await.contains(RESTRICTED));
    assert_eq!(f.head(d).await, Some((edit.id.as_bytes().to_vec(), f.home)));
    f.set_role(f.home, &member, "admin").await;
    f.accepted(&member, &to_private).await;

    // A delete is checked for the role, not the content it no longer has.
    f.set_role(f.private, &member, "member").await;
    let delete = f.subscription(&member, f.private, d, "delete", Some(&to_private), "");
    assert!(f.refused(&member, &delete).await.contains(RESTRICTED));
    let delete = f.subscription(&f.owner, f.private, d, "delete", Some(&to_private), "");
    f.accepted(&f.owner, &delete).await;
}
