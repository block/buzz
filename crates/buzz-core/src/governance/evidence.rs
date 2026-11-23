//! Signed evidence subjects. Eligibility and trust are resolved at acceptance.
use super::{document::UniqueJson, singleton, RepoCoordinate, MAX_DOCUMENT_BYTES};
use crate::{git_perms::RefPattern, kind::KIND_GIT_EVIDENCE};
use nostr::{Event, EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Immutable signed subject and statement, without cached actor standing.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// Community bound by the signature.
    pub community: Uuid,
    /// Exact original repository coordinate.
    pub repo: String,
    /// Full ref name.
    pub git_ref: String,
    /// Candidate commit object ID.
    pub head: String,
    /// Target commit the producer observed.
    pub base: String,
    /// Producer's statement.
    pub body: EvidenceBody,
}

/// Statements never carry the producer's own role or trust assertion.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceBody {
    /// Approval or withdrawal for precisely this subject.
    Approval {
        /// False withdraws this producer's approval.
        approved: bool,
    },
    /// A named check result for precisely this subject.
    Check {
        /// Name matched to the current trusted-producer policy.
        name: String,
        /// False supersedes an earlier passing result from this producer.
        passed: bool,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Content {
    version: u32,
    body: EvidenceBody,
}

impl Evidence {
    /// Validate a subject independently of authorization.
    pub fn validate(&self) -> Result<(), String> {
        self.repo.parse::<RepoCoordinate>()?;
        if self.git_ref.contains('*') {
            return Err("evidence requires an exact ref".into());
        }
        RefPattern::parse(&self.git_ref).map_err(|e| e.to_string())?;
        let oid = |s: &str| {
            matches!(s.len(), 40 | 64)
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if !oid(&self.head) || !oid(&self.base) || self.head.len() != self.base.len() {
            return Err("invalid evidence commit subject".into());
        }
        if let EvidenceBody::Check { name, .. } = &self.body {
            if name.is_empty()
                || name.len() > 128
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
            {
                return Err("invalid check name".into());
            }
        }
        Ok(())
    }

    /// Verify the producer signature and exact, unambiguous subject binding.
    pub fn parse(event: &Event) -> Result<Self, String> {
        if event.kind.as_u16() as u32 != KIND_GIT_EVIDENCE {
            return Err("wrong evidence kind".into());
        }
        event.verify().map_err(|_| "invalid evidence signature")?;
        if event.content.len() > MAX_DOCUMENT_BYTES {
            return Err("evidence too large".into());
        }
        for tag in event.tags.iter() {
            if !matches!(
                tag.as_slice().first().map(String::as_str),
                Some("community" | "a" | "ref" | "head" | "base" | "auth")
            ) {
                return Err("unsupported evidence tag".into());
            }
        }
        let mut decoder = serde_json::Deserializer::from_str(&event.content);
        UniqueJson::deserialize(&mut decoder).map_err(|e| e.to_string())?;
        decoder.end().map_err(|e| e.to_string())?;
        let content: Content = serde_json::from_str(&event.content).map_err(|e| e.to_string())?;
        if content.version != 1 {
            return Err("unsupported evidence version".into());
        }
        let result = Self {
            community: singleton(event, "community")?
                .parse()
                .map_err(|_| "invalid community")?,
            repo: singleton(event, "a")?.into(),
            git_ref: singleton(event, "ref")?.into(),
            head: singleton(event, "head")?.into(),
            base: singleton(event, "base")?.into(),
            body: content.body,
        };
        result.validate()?;
        Ok(result)
    }

    /// Construct the event for signing by the actual producer.
    pub fn builder(&self) -> Result<EventBuilder, String> {
        self.validate()?;
        let tags = [
            ("community", self.community.to_string()),
            ("a", self.repo.clone()),
            ("ref", self.git_ref.clone()),
            ("head", self.head.clone()),
            ("base", self.base.clone()),
        ]
        .into_iter()
        .map(|(k, v)| Tag::parse([k.to_string(), v]).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
        let content = serde_json::to_string(&Content {
            version: 1,
            body: self.body.clone(),
        })
        .map_err(|e| e.to_string())?;
        Ok(EventBuilder::new(Kind::Custom(KIND_GIT_EVIDENCE as u16), content).tags(tags))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn subject() -> Evidence {
        Evidence {
            community: Uuid::new_v4(),
            repo: format!("30617:{}:repo", Keys::generate().public_key().to_hex()),
            git_ref: "refs/heads/main".into(),
            head: "a".repeat(40),
            base: "b".repeat(40),
            body: EvidenceBody::Approval { approved: true },
        }
    }

    #[test]
    fn evidence_signed_subject_is_complete_and_unambiguous() {
        let keys = Keys::generate();
        let subject = subject();
        let event = subject.builder().unwrap().sign_with_keys(&keys).unwrap();
        assert_eq!(Evidence::parse(&event).unwrap(), subject);
        for name in ["community", "a", "ref", "head", "base"] {
            let original: Vec<Tag> = event.tags.iter().cloned().collect();
            let missing = original
                .iter()
                .filter(|t| t.as_slice()[0] != name)
                .cloned()
                .collect::<Vec<_>>();
            let mut duplicate = original.clone();
            duplicate.push(
                original
                    .iter()
                    .find(|t| t.as_slice()[0] == name)
                    .unwrap()
                    .clone(),
            );
            for tags in [missing, duplicate] {
                let changed = EventBuilder::new(event.kind, event.content.clone())
                    .tags(tags)
                    .sign_with_keys(&keys)
                    .unwrap();
                assert!(Evidence::parse(&changed).is_err(), "{name}");
            }
        }
        let mut tampered = event;
        tampered.content = tampered.content.replace("true", "false");
        assert!(Evidence::parse(&tampered).is_err());
    }

    #[test]
    fn evidence_rejects_ambiguous_json_and_wildcard_subjects() {
        let keys = Keys::generate();
        let mut subject = subject();
        let event = subject.builder().unwrap().sign_with_keys(&keys).unwrap();
        for content in [
            r#"{"version":1,"version":1,"body":{"type":"approval","approved":true}}"#,
            r#"{"version":2,"body":{"type":"approval","approved":true}}"#,
            r#"{"version":1,"body":{"type":"approval","approved":true,"standing":"admin"}}"#,
            r#"{"version":1,"body":{"type":"approval","approved":true,"approved":false}}"#,
        ] {
            let changed = EventBuilder::new(event.kind, content)
                .tags(event.tags.iter().cloned())
                .sign_with_keys(&keys)
                .unwrap();
            assert!(Evidence::parse(&changed).is_err());
        }
        subject.git_ref = "refs/heads/*".into();
        assert!(subject.builder().is_err());
    }

    #[test]
    fn evidence_preserves_withdrawals_and_failing_checks() {
        let keys = Keys::generate();
        for body in [
            EvidenceBody::Approval { approved: false },
            EvidenceBody::Check {
                name: "ci/test".into(),
                passed: false,
            },
        ] {
            let subject = Evidence { body, ..subject() };
            let event = subject.builder().unwrap().sign_with_keys(&keys).unwrap();
            assert_eq!(Evidence::parse(&event).unwrap(), subject);
        }
    }
}
