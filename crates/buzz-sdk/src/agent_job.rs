//! Parsing and validation helpers for durable agent-job events.

pub use buzz_core::agent_job::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builders::{
        build_agent_job_accepted, build_agent_job_cancel, build_agent_job_error,
        build_agent_job_progress, build_agent_job_request, build_agent_job_result,
    };
    use buzz_core::{
        agent_job::{
            AgentJobAccepted, AgentJobAcceptedState, AgentJobCancel, AgentJobError,
            AgentJobErrorState, AgentJobProgress, AgentJobProgressState, AgentJobRequest,
            AgentJobResult, AgentJobResultState, JobArtifact, AGENT_JOB_SCHEMA,
            MAX_AGENT_JOB_CONTENT_BYTES, MAX_JOB_ARGV_ENTRIES, MAX_JOB_ARTIFACTS,
            MAX_JOB_ARTIFACT_NAME_BYTES, MAX_JOB_ARTIFACT_URI_BYTES, MAX_JOB_CWD_BYTES,
            MAX_JOB_REASON_BYTES, MAX_JOB_SUMMARY_BYTES,
        },
        kind::{
            KIND_JOB_ACCEPTED, KIND_JOB_CANCEL, KIND_JOB_ERROR, KIND_JOB_PROGRESS,
            KIND_JOB_REQUEST, KIND_JOB_RESULT,
        },
    };
    use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag};
    use uuid::Uuid;

    struct Fixture {
        requester: Keys,
        target: Keys,
        channel: Uuid,
        job: Uuid,
        request_id: EventId,
    }

    fn fixture() -> Fixture {
        Fixture {
            requester: Keys::generate(),
            target: Keys::generate(),
            channel: Uuid::new_v4(),
            job: Uuid::new_v4(),
            request_id: EventId::from_byte_array([7; 32]),
        }
    }

    fn request_payload() -> AgentJobRequest {
        AgentJobRequest {
            schema: AGENT_JOB_SCHEMA,
            driver: "lh".into(),
            argv: vec!["lockdown".into(), "run".into()],
            cwd: "/tmp/workspace".into(),
            summary: "Run governed work".into(),
        }
    }

    fn artifact() -> JobArtifact {
        JobArtifact {
            name: "receipt".into(),
            uri: "file:///tmp/receipt.json".into(),
            sha256: Some("a".repeat(64)),
        }
    }

    fn accepted(job: Uuid) -> AgentJobAccepted {
        AgentJobAccepted {
            schema: AGENT_JOB_SCHEMA,
            job,
            attempt: 1,
            state: AgentJobAcceptedState::Accepted,
            accepted_at: "2023-11-14T22:13:20Z".parse().unwrap(),
        }
    }

    fn progress(job: Uuid) -> AgentJobProgress {
        AgentJobProgress {
            schema: AGENT_JOB_SCHEMA,
            job,
            attempt: 1,
            seq: 3,
            state: AgentJobProgressState::Running,
            summary: "Still running".into(),
            artifacts: vec![artifact()],
        }
    }

    fn result(job: Uuid) -> AgentJobResult {
        AgentJobResult {
            schema: AGENT_JOB_SCHEMA,
            job,
            attempt: 1,
            state: AgentJobResultState::Succeeded,
            exit_code: 0,
            summary: "Done".into(),
            artifacts: vec![artifact()],
            finished_at: "2023-11-14T22:15:00Z".parse().unwrap(),
        }
    }

    fn cancel(job: Uuid) -> AgentJobCancel {
        AgentJobCancel {
            schema: AGENT_JOB_SCHEMA,
            job,
            reason: "No longer needed".into(),
        }
    }

    fn error(job: Uuid) -> AgentJobError {
        AgentJobError {
            schema: AGENT_JOB_SCHEMA,
            job,
            attempt: 1,
            state: AgentJobErrorState::Failed,
            code: "driver_failed".into(),
            summary: "Driver failed".into(),
            retryable: true,
            artifacts: vec![artifact()],
            finished_at: "2023-11-14T22:15:00Z".parse().unwrap(),
        }
    }

    fn sign(builder: EventBuilder, keys: &Keys) -> Event {
        builder.sign_with_keys(keys).unwrap()
    }

    fn raw_event(kind: u32, content: String, tags: Vec<Vec<String>>, keys: &Keys) -> Event {
        let tags = tags
            .into_iter()
            .map(|parts| Tag::parse(parts).unwrap())
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags(tags)
            .allow_self_tagging()
            .sign_with_keys(keys)
            .unwrap()
    }

    fn base_tags(f: &Fixture) -> Vec<Vec<String>> {
        vec![
            vec!["h".into(), f.channel.to_string()],
            vec!["p".into(), f.target.public_key().to_hex()],
            vec!["job".into(), f.job.to_string()],
        ]
    }

    fn assert_tag_shape(event: &Event, expected: &[&str]) {
        let actual = event
            .tags
            .iter()
            .map(|tag| {
                let parts = tag.as_slice();
                assert_eq!(parts.len(), 2);
                parts[0].as_str()
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn every_kind_builds_and_round_trips_with_canonical_tags() {
        let f = fixture();
        let parent = Uuid::new_v4();
        let source = EventId::from_byte_array([9; 32]);
        let request = sign(
            build_agent_job_request(
                f.channel,
                f.target.public_key(),
                f.job,
                Some(source),
                Some(parent),
                &request_payload(),
            )
            .unwrap(),
            &f.requester,
        );
        assert_tag_shape(&request, &["h", "p", "job", "e", "parent-job"]);
        let parsed = parse_agent_job_event(&request).unwrap();
        assert_eq!(parsed.kind, KIND_JOB_REQUEST);
        assert_eq!(parsed.job, f.job);
        assert_eq!(parsed.linked_event_id, Some(source));
        assert_eq!(parsed.parent_job, Some(parent));
        assert!(matches!(parsed.payload, AgentJobPayload::Request(_)));

        let accepted = sign(
            build_agent_job_accepted(
                f.channel,
                f.requester.public_key(),
                f.request_id,
                &accepted(f.job),
            )
            .unwrap(),
            &f.target,
        );
        assert_tag_shape(&accepted, &["h", "p", "job", "e"]);
        assert!(matches!(
            parse_agent_job_event(&accepted).unwrap().payload,
            AgentJobPayload::Accepted(_)
        ));

        let progress = sign(
            build_agent_job_progress(
                f.channel,
                f.requester.public_key(),
                f.request_id,
                &progress(f.job),
            )
            .unwrap(),
            &f.target,
        );
        assert_tag_shape(&progress, &["h", "p", "job", "e", "seq"]);
        let parsed = parse_agent_job_event(&progress).unwrap();
        assert_eq!(parsed.seq, Some(3));
        assert!(matches!(parsed.payload, AgentJobPayload::Progress(_)));

        let result = sign(
            build_agent_job_result(
                f.channel,
                f.requester.public_key(),
                f.request_id,
                &result(f.job),
            )
            .unwrap(),
            &f.target,
        );
        assert_tag_shape(&result, &["h", "p", "job", "e"]);
        assert!(parse_agent_job_event(&result)
            .unwrap()
            .payload
            .is_terminal());

        let cancel = sign(
            build_agent_job_cancel(
                f.channel,
                f.target.public_key(),
                f.request_id,
                &cancel(f.job),
            )
            .unwrap(),
            &f.requester,
        );
        assert_tag_shape(&cancel, &["h", "p", "job", "e"]);
        assert!(matches!(
            parse_agent_job_event(&cancel).unwrap().payload,
            AgentJobPayload::Cancel(_)
        ));

        let error = sign(
            build_agent_job_error(
                f.channel,
                f.requester.public_key(),
                f.request_id,
                &error(f.job),
            )
            .unwrap(),
            &f.target,
        );
        assert_tag_shape(&error, &["h", "p", "job", "e"]);
        assert!(parse_agent_job_event(&error).unwrap().payload.is_terminal());
    }

    #[test]
    fn strict_json_rejects_unknown_fields_and_invalid_state() {
        let f = fixture();
        let mut tags = base_tags(&f);
        let unknown = serde_json::json!({
            "schema": 1, "driver": "lh", "argv": [], "cwd": "/tmp",
            "summary": "x", "extra": true
        })
        .to_string();
        let event = raw_event(KIND_JOB_REQUEST, unknown, tags.clone(), &f.requester);
        assert!(matches!(
            parse_agent_job_event(&event),
            Err(AgentJobValidationError::InvalidContent(_))
        ));

        tags.push(vec!["e".into(), f.request_id.to_hex()]);
        tags.push(vec!["seq".into(), "1".into()]);
        let invalid_state = serde_json::json!({
            "schema": 1, "job": f.job, "attempt": 1, "seq": 1,
            "state": "paused", "summary": "x", "artifacts": []
        })
        .to_string();
        let event = raw_event(KIND_JOB_PROGRESS, invalid_state, tags, &f.target);
        assert!(matches!(
            parse_agent_job_event(&event),
            Err(AgentJobValidationError::InvalidContent(_))
        ));
    }

    #[test]
    fn request_and_artifact_aggregate_bounds_are_enforced() {
        let mut request = request_payload();
        request.argv = vec!["x".into(); MAX_JOB_ARGV_ENTRIES + 1];
        assert!(request.validate().is_err());

        request.argv = vec!["x".repeat(8 * 1024 + 1)];
        assert!(request.validate().is_err());
        request.argv = vec!["x".repeat(8 * 1024); 9];
        assert!(request.validate().is_err());

        let mut progress = progress(Uuid::new_v4());
        progress.artifacts = vec![artifact(); MAX_JOB_ARTIFACTS + 1];
        assert!(progress.validate().is_err());

        progress.artifacts = vec![JobArtifact {
            name: "receipt".into(),
            uri: "file:///tmp/receipt".into(),
            sha256: Some("A".repeat(64)),
        }];
        assert!(progress.validate().is_err());

        progress.artifacts = vec![JobArtifact {
            name: "x".repeat(MAX_JOB_ARTIFACT_NAME_BYTES + 1),
            uri: "file:///tmp/receipt".into(),
            sha256: None,
        }];
        assert!(progress.validate().is_err());

        progress.artifacts = vec![JobArtifact {
            name: "receipt".into(),
            uri: "x".repeat(MAX_JOB_ARTIFACT_URI_BYTES + 1),
            sha256: None,
        }];
        assert!(progress.validate().is_err());

        request = request_payload();
        request.cwd = "x".repeat(MAX_JOB_CWD_BYTES + 1);
        assert!(request.validate().is_err());

        request = request_payload();
        request.summary = "x".repeat(MAX_JOB_SUMMARY_BYTES + 1);
        assert!(request.validate().is_err());

        let mut cancel = cancel(Uuid::new_v4());
        cancel.reason = "x".repeat(MAX_JOB_REASON_BYTES + 1);
        assert!(cancel.validate().is_err());
    }

    #[test]
    fn parser_rejects_oversized_content_duplicate_missing_and_bad_uuid() {
        let f = fixture();
        let oversized = raw_event(
            KIND_JOB_REQUEST,
            "x".repeat(MAX_AGENT_JOB_CONTENT_BYTES + 1),
            base_tags(&f),
            &f.requester,
        );
        assert!(matches!(
            parse_agent_job_event(&oversized),
            Err(AgentJobValidationError::ContentTooLarge { .. })
        ));

        let content = serde_json::to_string(&request_payload()).unwrap();
        let mut duplicate = base_tags(&f);
        duplicate.push(vec!["job".into(), f.job.to_string()]);
        assert!(matches!(
            parse_agent_job_event(&raw_event(
                KIND_JOB_REQUEST,
                content.clone(),
                duplicate,
                &f.requester
            )),
            Err(AgentJobValidationError::DuplicateTag(_))
        ));

        let missing = vec![
            vec!["h".into(), f.channel.to_string()],
            vec!["p".into(), f.target.public_key().to_hex()],
        ];
        assert!(matches!(
            parse_agent_job_event(&raw_event(
                KIND_JOB_REQUEST,
                content.clone(),
                missing,
                &f.requester
            )),
            Err(AgentJobValidationError::MissingTag("job"))
        ));

        let accepted_without_link = raw_event(
            KIND_JOB_ACCEPTED,
            serde_json::to_string(&accepted(f.job)).unwrap(),
            base_tags(&f),
            &f.target,
        );
        assert!(matches!(
            parse_agent_job_event(&accepted_without_link),
            Err(AgentJobValidationError::MissingTag("e"))
        ));

        let mut bad_uuid = base_tags(&f);
        bad_uuid[2][1] = "not-a-uuid".into();
        assert!(matches!(
            parse_agent_job_event(&raw_event(
                KIND_JOB_REQUEST,
                content,
                bad_uuid,
                &f.requester
            )),
            Err(AgentJobValidationError::InvalidTag(_))
        ));

        let mut mismatched_tags = base_tags(&f);
        mismatched_tags.push(vec!["e".into(), f.request_id.to_hex()]);
        let mismatched_payload = accepted(Uuid::new_v4());
        assert!(matches!(
            parse_agent_job_event(&raw_event(
                KIND_JOB_ACCEPTED,
                serde_json::to_string(&mismatched_payload).unwrap(),
                mismatched_tags,
                &f.target,
            )),
            Err(AgentJobValidationError::PayloadTagMismatch("job"))
        ));
    }

    #[test]
    fn parser_rejects_bad_or_mismatched_sequence() {
        let f = fixture();
        let content = serde_json::to_string(&progress(f.job)).unwrap();
        let mut tags = base_tags(&f);
        tags.push(vec!["e".into(), f.request_id.to_hex()]);
        tags.push(vec!["seq".into(), "03".into()]);
        assert!(matches!(
            parse_agent_job_event(&raw_event(
                KIND_JOB_PROGRESS,
                content.clone(),
                tags,
                &f.target
            )),
            Err(AgentJobValidationError::InvalidTag(_))
        ));

        let mut tags = base_tags(&f);
        tags.push(vec!["e".into(), f.request_id.to_hex()]);
        tags.push(vec!["seq".into(), "2".into()]);
        assert!(matches!(
            parse_agent_job_event(&raw_event(KIND_JOB_PROGRESS, content, tags, &f.target)),
            Err(AgentJobValidationError::PayloadTagMismatch("seq"))
        ));
    }

    #[test]
    fn caller_supplied_signer_and_link_expectations_are_enforced() {
        let f = fixture();
        let event = sign(
            build_agent_job_accepted(
                f.channel,
                f.requester.public_key(),
                f.request_id,
                &accepted(f.job),
            )
            .unwrap(),
            &f.target,
        );
        let expected = AgentJobEventExpectations {
            author: Some(f.target.public_key()),
            channel_id: Some(f.channel),
            peer: Some(f.requester.public_key()),
            linked_event_id: Some(f.request_id),
            job: Some(f.job),
        };
        validate_agent_job_event(&event, &expected).unwrap();

        let wrong = AgentJobEventExpectations {
            author: Some(Keys::generate().public_key()),
            ..expected
        };
        assert!(matches!(
            validate_agent_job_event(&event, &wrong),
            Err(AgentJobValidationError::ExpectationMismatch("author"))
        ));
    }

    #[test]
    fn parser_accepts_one_canonical_auth_tag_and_rejects_bad_auth_shapes() {
        let f = fixture();
        let content = serde_json::to_string(&request_payload()).unwrap();
        let auth = vec![
            "auth".into(),
            Keys::generate().public_key().to_hex(),
            "kind=43001&created_at>1".into(),
            "a".repeat(128),
        ];

        let mut tags = base_tags(&f);
        tags.push(auth.clone());
        let parsed = parse_agent_job_event(&raw_event(
            KIND_JOB_REQUEST,
            content.clone(),
            tags,
            &f.requester,
        ))
        .unwrap();
        assert_eq!(parsed.job, f.job);

        let mut duplicate = base_tags(&f);
        duplicate.push(auth.clone());
        duplicate.push(auth.clone());
        assert!(matches!(
            parse_agent_job_event(&raw_event(
                KIND_JOB_REQUEST,
                content.clone(),
                duplicate,
                &f.requester,
            )),
            Err(AgentJobValidationError::DuplicateTag(tag)) if tag == "auth"
        ));

        let mut malformed = base_tags(&f);
        malformed.push(vec![
            "auth".into(),
            Keys::generate().public_key().to_hex(),
            "kind=043001".into(),
            "a".repeat(128),
        ]);
        assert!(matches!(
            parse_agent_job_event(&raw_event(
                KIND_JOB_REQUEST,
                content,
                malformed,
                &f.requester,
            )),
            Err(AgentJobValidationError::InvalidTag(_))
        ));
    }

    #[test]
    fn all_kind_constants_are_covered() {
        assert_eq!(
            [
                KIND_JOB_REQUEST,
                KIND_JOB_ACCEPTED,
                KIND_JOB_PROGRESS,
                KIND_JOB_RESULT,
                KIND_JOB_CANCEL,
                KIND_JOB_ERROR
            ],
            [43001, 43002, 43003, 43004, 43005, 43006]
        );
    }
}
