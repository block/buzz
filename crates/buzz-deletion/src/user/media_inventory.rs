//! Disposable, disk-backed inventory. Never used as a restart journal.
use anyhow::{bail, Result};
use buzz_media::{ObjectVersionEntry, ObjectVersionKind};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
    Connection, SqliteConnection,
};

pub(super) const PAGE_SIZE: u16 = 500;
const MAX_PAGES: u32 = 262_144; // 1 GiB at 4096 bytes/page, including indexes.
const MAX_ENTRY_BYTES: usize = 16_384;

pub(super) struct Inventory {
    conn: SqliteConnection,
    // Private temporary directory; files are removed on success, error or cancellation.
    _directory: tempfile::TempDir,
}

impl Inventory {
    pub async fn new() -> Result<Self> {
        Self::with_page_limit(MAX_PAGES).await
    }

    async fn with_page_limit(max_pages: u32) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let options = SqliteConnectOptions::new()
            .filename(directory.path().join("inventory.sqlite"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Off)
            .synchronous(SqliteSynchronous::Off)
            .statement_cache_capacity(16)
            .pragma("page_size", "4096")
            .pragma("max_page_count", max_pages.to_string())
            .pragma("cache_size", "-4096")
            .pragma("mmap_size", "0")
            .pragma("temp_store", "FILE");
        let mut conn = SqliteConnection::connect_with(&options).await?;
        // Journaling is disabled because any failure discards this inventory; no
        // persisted decision is trusted after a restart. Source data stays intact
        // throughout collection. No application DB schema is changed.
        sqlx::raw_sql(
            "CREATE TABLE candidates (hash TEXT PRIMARY KEY, shared INTEGER NOT NULL DEFAULT 0) WITHOUT ROWID;
             CREATE TABLE owners (hash TEXT, pubkey TEXT, PRIMARY KEY(hash, pubkey)) WITHOUT ROWID;
             CREATE TABLE filenames (hash TEXT, name TEXT, PRIMARY KEY(hash, name)) WITHOUT ROWID;
             CREATE TABLE versions (id INTEGER PRIMARY KEY, hash TEXT NOT NULL,
                 priority INTEGER NOT NULL, unversioned INTEGER NOT NULL, entry TEXT NOT NULL);
             CREATE INDEX versions_by_hash ON versions(hash, priority, id);
             CREATE TABLE listing_markers (prefix TEXT, key TEXT, version TEXT,
                 PRIMARY KEY(prefix, key, version)) WITHOUT ROWID;",
        ).execute(&mut conn).await?;
        Ok(Self {
            conn,
            _directory: directory,
        })
    }

    pub async fn add_candidates(&mut self, hashes: &[String]) -> Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
        let mut tx = self.conn.begin().await?;
        for hash in hashes {
            sqlx::query("INSERT OR IGNORE INTO candidates(hash) VALUES (?)")
                .bind(hash)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn candidates(&mut self, after: &str) -> Result<Vec<(String, bool)>> {
        Ok(sqlx::query_as(
            "SELECT hash, shared FROM candidates WHERE hash > ? ORDER BY hash LIMIT ?",
        )
        .bind(after)
        .bind(i64::from(PAGE_SIZE))
        .fetch_all(&mut self.conn)
        .await?)
    }

    pub async fn contains(&mut self, hash: &str) -> Result<bool> {
        Ok(
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM candidates WHERE hash = ?)")
                .bind(hash)
                .fetch_one(&mut self.conn)
                .await?,
        )
    }

    pub async fn mark_shared(&mut self, hashes: &[String]) -> Result<()> {
        if hashes.is_empty() {
            return Ok(());
        }
        let mut tx = self.conn.begin().await?;
        for hash in hashes {
            sqlx::query("UPDATE candidates SET shared = 1 WHERE hash = ?")
                .bind(hash)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn add_versions(&mut self, versions: &[(String, ObjectVersionEntry)]) -> Result<()> {
        if versions.is_empty() {
            return Ok(());
        }
        let mut tx = self.conn.begin().await?;
        for (hash, version) in versions {
            let entry = serde_json::to_string(version)?;
            if entry.len() > MAX_ENTRY_BYTES {
                bail!("storage version metadata exceeds 16 KiB");
            }
            sqlx::query(
                "INSERT INTO versions(hash, priority, unversioned, entry) VALUES (?, ?, ?, ?)",
            )
            .bind(hash)
            .bind(priority(version))
            .bind(version.version_id.is_empty() || version.version_id == "null")
            .bind(entry)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn add_owner(&mut self, hash: &str, owner: &str) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO owners(hash, pubkey) VALUES (?, ?)")
            .bind(hash)
            .bind(owner)
            .execute(&mut self.conn)
            .await?;
        Ok(())
    }

    pub async fn owners(&mut self, hash: &str, after: &str) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT pubkey FROM owners WHERE hash = ? AND pubkey > ? ORDER BY pubkey LIMIT ?",
        )
        .bind(hash)
        .bind(after)
        .bind(i64::from(PAGE_SIZE))
        .fetch_all(&mut self.conn)
        .await?)
    }

    pub async fn add_filename(&mut self, hash: &str, name: &str) -> Result<()> {
        if name.len() > MAX_ENTRY_BYTES {
            bail!("stored filename exceeds 16 KiB");
        }
        sqlx::query("INSERT OR IGNORE INTO filenames(hash, name) VALUES (?, ?)")
            .bind(hash)
            .bind(name)
            .execute(&mut self.conn)
            .await?;
        Ok(())
    }

    // A hash may have several stored extensions. Use the first for the display
    // name; every actual key remains available in the file's versions array.
    pub async fn filename(&mut self, hash: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT name FROM filenames WHERE hash = ? ORDER BY name LIMIT 1")
                .bind(hash)
                .fetch_optional(&mut self.conn)
                .await?,
        )
    }

    pub async fn require_immutable_versions(&mut self) -> Result<()> {
        let invalid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM versions v JOIN candidates c ON c.hash = v.hash
             WHERE c.shared = 0 AND v.unversioned = 1)",
        )
        .fetch_one(&mut self.conn)
        .await?;
        if invalid {
            bail!("selected media has unversioned objects; immutable version IDs are required");
        }
        Ok(())
    }

    pub async fn remember_marker(
        &mut self,
        prefix: &str,
        key: &str,
        version: &str,
    ) -> Result<bool> {
        Ok(sqlx::query(
            "INSERT OR IGNORE INTO listing_markers(prefix, key, version) VALUES (?, ?, ?)",
        )
        .bind(prefix)
        .bind(key)
        .bind(version)
        .execute(&mut self.conn)
        .await?
        .rows_affected()
            == 1)
    }

    pub async fn versions(
        &mut self,
        hash: &str,
        after: (i64, i64),
    ) -> Result<Vec<(i64, i64, ObjectVersionEntry)>> {
        let rows: Vec<(i64, i64, String)> = sqlx::query_as(
            "SELECT priority, id, entry FROM versions
             WHERE hash = ? AND (priority, id) > (?, ?) ORDER BY priority, id LIMIT ?",
        )
        .bind(hash)
        .bind(after.0)
        .bind(after.1)
        .bind(i64::from(PAGE_SIZE))
        .fetch_all(&mut self.conn)
        .await?;
        rows.into_iter()
            .map(|(priority, id, entry)| Ok((priority, id, serde_json::from_str(&entry)?)))
            .collect()
    }

    pub async fn close(self) -> Result<()> {
        self.conn.close().await?;
        Ok(())
    }
}

fn priority(version: &ObjectVersionEntry) -> i64 {
    match (version.key.starts_with("_uploads/"), version.kind) {
        (false, ObjectVersionKind::Object) => 0,
        (false, ObjectVersionKind::DeleteMarker) => 1,
        (true, ObjectVersionKind::DeleteMarker) => 2,
        (true, ObjectVersionKind::Object) => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn inventory_deduplicates_pages_and_orders_attribution_last() {
        let mut inventory = Inventory::new().await.unwrap();
        let path = inventory._directory.path().to_path_buf();
        let hashes: Vec<_> = (0..501).map(|i| format!("{i:064x}")).collect();
        inventory.add_candidates(&hashes).await.unwrap();
        inventory.add_candidates(&hashes).await.unwrap();
        let page = inventory.candidates("").await.unwrap();
        assert_eq!(page.len(), 500);
        assert_eq!(inventory.candidates(&page[499].0).await.unwrap().len(), 1);
        let hash = &hashes[0];
        let entries: Vec<_> = [
            ("_uploads/claim", ObjectVersionKind::Object),
            ("_uploads/claim", ObjectVersionKind::DeleteMarker),
            ("blob", ObjectVersionKind::DeleteMarker),
            ("blob", ObjectVersionKind::Object),
        ]
        .into_iter()
        .map(|(key, kind)| {
            (
                hash.clone(),
                ObjectVersionEntry {
                    key: key.into(),
                    version_id: "v1".into(),
                    kind,
                    size: 1,
                },
            )
        })
        .collect();
        inventory.add_versions(&entries).await.unwrap();
        let ordered = inventory.versions(hash, (-1, 0)).await.unwrap();
        assert_eq!(
            ordered.iter().map(|v| v.0).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(ordered[3].2.key, "_uploads/claim");
        // A large file family must cross a page without losing a version or
        // moving its final ownership record ahead of the remaining bytes.
        let many: Vec<_> = (0..501)
            .map(|i| {
                (
                    hashes[1].clone(),
                    ObjectVersionEntry {
                        key: "blob".into(),
                        version_id: format!("v{i}"),
                        kind: ObjectVersionKind::Object,
                        size: 1,
                    },
                )
            })
            .chain(std::iter::once((hashes[1].clone(), entries[0].1.clone())))
            .collect();
        inventory.add_versions(&many).await.unwrap();
        let first = inventory.versions(&hashes[1], (-1, 0)).await.unwrap();
        assert_eq!(first.len(), 500);
        let last = first.last().unwrap();
        let rest = inventory
            .versions(&hashes[1], (last.0, last.1))
            .await
            .unwrap();
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[1].2.key, "_uploads/claim");
        assert!(inventory
            .remember_marker("_uploads/", "key", "v1")
            .await
            .unwrap());
        assert!(inventory
            .remember_marker("_uploads/", "key", "v2")
            .await
            .unwrap());
        assert!(!inventory
            .remember_marker("_uploads/", "key", "v1")
            .await
            .unwrap());
        assert!(inventory.remember_marker("", "key", "v1").await.unwrap());
        inventory.close().await.unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn disk_limit_aborts_instead_of_truncating_inventory() {
        let mut inventory = Inventory::with_page_limit(8).await.unwrap();
        let version = ObjectVersionEntry {
            key: "k".repeat(8000),
            version_id: "v1".into(),
            kind: ObjectVersionKind::Object,
            size: 1,
        };
        let mut full = false;
        for _ in 0..16 {
            if inventory
                .add_versions(&[("a".repeat(64), version.clone())])
                .await
                .is_err()
            {
                full = true;
                break;
            }
        }
        assert!(full);
        let size = std::fs::metadata(inventory._directory.path().join("inventory.sqlite"))
            .unwrap()
            .len();
        assert!(size <= 8 * 4096);
    }
}
