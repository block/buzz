use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use buzz_sdk::reminders::Reminder;
use sha2::{Digest, Sha256};

pub(super) struct Receipts {
    directory: PathBuf,
    _lock: File,
}

impl Receipts {
    pub(super) fn open(base: &Path, relay: &str, author: &str) -> Result<Self> {
        let scope = hex::encode(Sha256::digest(format!("{relay}\n{author}")));
        let directory = base.join(scope);
        fs::create_dir_all(&directory).context("create reminder receipt directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let lock = private_file(&directory.join("lock"), false)?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("another harness owns reminder delivery for this identity")?;
        Ok(Self {
            directory,
            _lock: lock,
        })
    }

    fn path(&self, reminder: &Reminder) -> PathBuf {
        self.directory
            .join(hex::encode(Sha256::digest(&reminder.id)))
    }

    pub(super) fn contains(&self, reminder: &Reminder) -> Result<bool> {
        let mut value = String::new();
        match File::open(self.path(reminder)) {
            Ok(mut file) => {
                file.read_to_string(&mut value)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
        let value = value.trim();
        anyhow::ensure!(
            value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid reminder delivery receipt"
        );
        Ok(value == reminder.event_id)
    }

    pub(super) fn record(&self, reminder: &Reminder) -> Result<()> {
        let temporary = self
            .directory
            .join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let mut file = private_file(&temporary, true)?;
        writeln!(file, "{}", reminder.event_id)?;
        file.sync_all()?;
        fs::rename(&temporary, self.path(reminder))?;
        #[cfg(unix)]
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}

fn private_file(path: &Path, exclusive: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if exclusive {
        options.create_new(true);
    } else {
        options.create(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
