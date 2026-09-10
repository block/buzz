//! Bounded macOS process-group evidence for an EPERM signal result.

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use std::collections::HashSet;
use std::io::{self, Read};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
const QUERY_DEADLINE: Duration = Duration::from_millis(250);
const QUERY_POLL: Duration = Duration::from_millis(5);

#[derive(Default)]
pub(super) struct Probe {
    // A timed-out query stays owned until a later cleanup reaps it.
    child: Option<Child>,
    #[cfg(test)]
    defer_reaping: bool,
}

impl Probe {
    pub(super) fn reap(&mut self) -> bool {
        #[cfg(test)]
        if self.defer_reaping {
            return false;
        }
        let Some(child) = self.child.as_mut() else {
            return true;
        };
        let _ = child.kill();
        match child.try_wait() {
            Ok(Some(_)) => {
                self.child = None;
                true
            }
            Ok(None) | Err(_) => false,
        }
    }

    pub(super) fn all_zombies(&mut self, leader: u32) -> bool {
        if !self.reap() {
            return false;
        }
        let deadline = Instant::now() + QUERY_DEADLINE;
        let child = Command::new("/bin/ps")
            .args(["-axo", "pid=,pgid=,stat="])
            .env_clear()
            .env("LANG", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(child) = child else {
            return false;
        };
        self.child = Some(child);
        let result = self
            .read_snapshot(deadline)
            .is_some_and(|bytes| all_group_members_zombie(&bytes, leader));
        if !result {
            self.reap();
        }
        result
    }

    fn read_snapshot(&mut self, deadline: Instant) -> Option<Vec<u8>> {
        let child = self.child.as_mut()?;
        let mut stdout = child.stdout.take()?;
        let flags = OFlag::from_bits_truncate(fcntl(&stdout, FcntlArg::F_GETFL).ok()?);
        fcntl(&stdout, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).ok()?;
        let mut output = Vec::new();
        let mut buffer = [0_u8; 4096];
        let mut eof = false;
        loop {
            let mut blocked = false;
            match stdout.read(&mut buffer) {
                Ok(0) => eof = true,
                Ok(count) => {
                    if output.len() + count > MAX_SNAPSHOT_BYTES {
                        return None;
                    }
                    output.extend_from_slice(&buffer[..count]);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => blocked = true,
                Err(_) => return None,
            }
            if eof {
                if let Some(status) = child.try_wait().ok()? {
                    self.child = None;
                    return status.success().then_some(output);
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            if eof || blocked {
                std::thread::sleep(
                    QUERY_POLL.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
    }
}

fn all_group_members_zombie(bytes: &[u8], leader: u32) -> bool {
    if bytes.is_empty() || bytes.len() > MAX_SNAPSHOT_BYTES || !bytes.ends_with(b"\n") {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut leader_found = false;
    let mut process_ids = HashSet::new();
    for line in text.lines() {
        let mut columns = line.split_whitespace();
        let Some(process_id) = columns
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|id| *id > 0)
        else {
            return false;
        };
        let Some(group_id) = columns
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|id| *id > 0)
        else {
            return false;
        };
        let Some(status) = columns.next() else {
            return false;
        };
        if columns.next().is_some() || !process_ids.insert(process_id) {
            return false;
        }
        if group_id == leader {
            if !status.starts_with('Z') {
                return false;
            }
            if process_id == leader {
                leader_found = true;
            }
        }
    }
    leader_found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_complete_positive_evidence_for_every_group_member() {
        for snapshot in ["10 10 Z\n", "10 10 Z+\n11 10 Z\n20 20 S\n"] {
            assert!(
                all_group_members_zombie(snapshot.as_bytes(), 10),
                "{snapshot}"
            );
        }
        for snapshot in [
            "",
            "10 10 Z",
            "11 10 Z\n",
            "10 10 Z\n11 10 S\n",
            "10 10 Z\n11 10 T\n",
            "10 10 Z\n11 10 ?\n",
            "10 11 Z\n",
            "10 10 Z extra\n",
            "10 10 Z\ninvalid\n",
            "10 10 Z\n10 10 Z\n",
            "0 10 Z\n",
        ] {
            assert!(
                !all_group_members_zombie(snapshot.as_bytes(), 10),
                "{snapshot:?}"
            );
        }
    }

    struct QueryFixture(Probe);

    impl QueryFixture {
        fn new(script: &str) -> Self {
            let child = Command::new("/bin/sh")
                .args(["-c", script])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            Self(Probe {
                child: Some(child),
                ..Probe::default()
            })
        }
    }

    impl Drop for QueryFixture {
        fn drop(&mut self) {
            self.0.defer_reaping = false;
            let deadline = Instant::now() + Duration::from_secs(2);
            while !self.0.reap() && Instant::now() < deadline {
                std::thread::yield_now();
            }
            if !std::thread::panicking() {
                assert!(self.0.child.is_none(), "query fixture must be reaped");
            }
        }
    }

    #[test]
    fn oversized_query_output_is_rejected_and_query_stays_owned() {
        let mut fixture = QueryFixture::new("exec /bin/dd if=/dev/zero bs=4096 count=65");
        assert!(fixture
            .0
            .read_snapshot(Instant::now() + Duration::from_secs(2))
            .is_none());
        assert!(fixture.0.child.is_some());
        assert!(!all_group_members_zombie(
            &vec![b'Z'; MAX_SNAPSHOT_BYTES + 1],
            10
        ));
    }

    #[test]
    fn stalled_query_is_retained_and_cannot_be_replaced_before_reaping() {
        let mut fixture = QueryFixture::new("exec sleep 60");
        let original_id = fixture.0.child.as_ref().unwrap().id();
        assert!(fixture.0.read_snapshot(Instant::now()).is_none());
        assert_eq!(fixture.0.child.as_ref().unwrap().id(), original_id);
        fixture.0.defer_reaping = true;
        assert!(!fixture.0.all_zombies(10));
        assert_eq!(fixture.0.child.as_ref().unwrap().id(), original_id);
        fixture.0.defer_reaping = false;
        let deadline = Instant::now() + Duration::from_secs(2);
        while !fixture.0.reap() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(fixture.0.child.is_none());
        assert!(!fixture.0.all_zombies(u32::MAX));
    }

    #[test]
    fn nonzero_query_exit_is_not_complete_evidence() {
        let mut fixture = QueryFixture::new("printf '10 10 Z\\n'; exit 7");
        assert!(fixture
            .0
            .read_snapshot(Instant::now() + Duration::from_secs(2))
            .is_none());
        assert!(fixture.0.child.is_none());
    }
}
