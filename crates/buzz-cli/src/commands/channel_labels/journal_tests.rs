use super::*;
use std::io::Write;

fn record() -> CommandRecord {
    CommandRecord {
        version: 1,
        relay_url: "https://tenant.example".into(),
        relay_key: Keys::generate().public_key(),
        event: EventBuilder::new(Kind::Custom(9002), "")
            .tags([
                Tag::parse(["h", &Uuid::new_v4().to_string()]).unwrap(),
                Tag::parse(["add-label", "a"]).unwrap(),
            ])
            .sign_with_keys(&Keys::generate())
            .unwrap(),
    }
}

#[test]
fn journal_is_exclusive_private_and_preserves_exact_signed_event() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("command.jsonl");
    let mut journal = Journal::create(&path, record()).unwrap();
    let event = journal.record.event.clone();
    assert!(Journal::create(&path, record()).is_err());
    assert!(Journal::open(&path).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    journal.persist(CommandOutcome::Unknown).unwrap();
    drop(journal);
    let journal = Journal::open(&path).unwrap();
    assert_eq!(journal.record.event, event);
    assert_eq!(journal.outcome, CommandOutcome::Unknown);
}

#[test]
fn every_partial_outcome_append_recovers_conservatively_without_resigning() {
    for prior in [
        CommandOutcome::Prepared,
        CommandOutcome::Unknown,
        CommandOutcome::Committed,
    ] {
        for append in [
            b"\"unknown\"\n".as_slice(),
            b"\"rejected\"\n",
            b"\"committed\"\n",
        ] {
            for length in 1..append.len() {
                let temp = tempfile::tempdir().unwrap();
                let path = temp.path().join("command.jsonl");
                let mut journal = Journal::create(&path, record()).unwrap();
                let event = journal.record.event.clone();
                journal.persist(prior).unwrap();
                drop(journal);
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap()
                    .write_all(&append[..length])
                    .unwrap();
                let journal = Journal::open(&path).unwrap();
                assert_eq!(journal.record.event, event);
                assert_eq!(journal.outcome, prior.before_send());
                drop(journal);
                assert_eq!(Journal::open(&path).unwrap().outcome, prior.before_send());
            }
        }
    }
}

#[test]
fn journal_rejects_incomplete_command_or_corrupt_complete_records() {
    for suffix in ["\"committed\"\n\"rejected\"\n", "not-json\n"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("command.jsonl");
        drop(Journal::create(&path, record()).unwrap());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(suffix.as_bytes())
            .unwrap();
        assert!(Journal::open(&path).is_err());
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("command.jsonl");
    std::fs::write(&path, b"{\"version\":1").unwrap();
    assert!(Journal::open(&path).is_err());
}

#[test]
fn journal_size_limit_preserves_record_and_refuses_new_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("command.jsonl");
    let mut journal = Journal::create(&path, record()).unwrap();
    // Simulate a full file without an unbounded retry loop.
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(1024 * 1024).unwrap();
    assert!(journal.persist(CommandOutcome::Unknown).is_err());
    assert_eq!(journal.outcome, CommandOutcome::Prepared);
    file.set_len(1024 * 1024 + 1).unwrap();
    drop(journal);
    assert!(Journal::open(&path).is_err());
}
