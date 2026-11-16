//! Disposable, bounded in-memory inventory. Never used as a restart journal.
use anyhow::{bail, Context, Result};
use buzz_media::{ObjectVersionEntry, ObjectVersionKind};
use std::collections::{BTreeMap, BTreeSet};
use std::mem::size_of;
use std::ops::Bound::{Excluded, Included, Unbounded};

pub(super) const PAGE_SIZE: u16 = 500;
const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 250_000;
const MAX_ENTRY_BYTES: usize = 16_384;
// Conservative allowance per tree entry for node pointers, spare node slots,
// and allocator overhead, in addition to inline data and owned string bytes.
// This is an inventory accounting budget, not a process RSS limit. Transient
// listing/report pages and the DB/HTTP clients have their own allocations.
const TREE_ENTRY_OVERHEAD: usize = 256;

type VersionKey = (String, i64, i64);
type Marker = (String, String, String);

#[derive(Default)]
struct Candidate {
    shared: bool,
    db_checked: bool,
}

pub(super) struct Inventory {
    candidates: BTreeMap<String, Candidate>,
    filenames: BTreeSet<(String, String)>,
    versions: BTreeMap<VersionKey, ObjectVersionEntry>,
    markers: BTreeSet<Marker>,
    bytes: usize,
    entries: usize,
    max_bytes: usize,
    max_entries: usize,
    next_id: i64,
}

impl Inventory {
    pub async fn new() -> Result<Self> {
        Ok(Self::with_limits(MAX_BYTES, MAX_ENTRIES))
    }

    pub(super) fn with_limits(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            candidates: BTreeMap::new(),
            filenames: BTreeSet::new(),
            versions: BTreeMap::new(),
            markers: BTreeSet::new(),
            bytes: 0,
            entries: 0,
            max_bytes,
            max_entries,
            next_id: 0,
        }
    }

    // Charge before retaining any new data. On failure the caller discards the
    // entire inventory, before report::execute can mutate storage or attribution.
    fn reserve<T>(&mut self, string_bytes: usize) -> Result<()> {
        let bytes = self
            .bytes
            .checked_add(size_of::<T>() + TREE_ENTRY_OVERHEAD)
            .and_then(|bytes| bytes.checked_add(string_bytes))
            .context("media inventory byte accounting overflow")?;
        if bytes > self.max_bytes || self.entries >= self.max_entries {
            bail!("media inventory capacity exceeded ({} byte budget, {} entries); discovery aborted before deletion", self.max_bytes, self.max_entries);
        }
        self.bytes = bytes;
        self.entries += 1;
        Ok(())
    }

    pub async fn add_candidates(&mut self, hashes: &[String]) -> Result<()> {
        for hash in hashes {
            check_string(hash)?;
            if !self.candidates.contains_key(hash) {
                self.reserve::<(String, Candidate)>(hash.len())?;
                self.candidates.insert(hash.clone(), Candidate::default());
            }
        }
        Ok(())
    }

    pub async fn candidates(&mut self, after: &str) -> Result<Vec<(String, bool)>> {
        Ok(self
            .candidates
            .range::<str, _>((Excluded(after), Unbounded))
            .take(usize::from(PAGE_SIZE))
            .map(|(hash, candidate)| (hash.clone(), candidate.shared))
            .collect())
    }

    pub async fn add_classified_candidates(&mut self, page: &[(String, bool)]) -> Result<()> {
        for (hash, shared) in page {
            self.add_candidates(std::slice::from_ref(hash)).await?;
            if let Some(candidate) = self.candidates.get_mut(hash) {
                candidate.shared |= shared;
                candidate.db_checked = true;
            }
        }
        Ok(())
    }

    pub async fn unchecked_candidates(&mut self, after: &str) -> Result<Vec<String>> {
        Ok(self
            .candidates
            .range::<str, _>((Excluded(after), Unbounded))
            .filter(|(_, candidate)| !candidate.db_checked)
            .take(usize::from(PAGE_SIZE))
            .map(|(hash, _)| hash.clone())
            .collect())
    }

    pub async fn contains(&mut self, hash: &str) -> Result<bool> {
        Ok(self.candidates.contains_key(hash))
    }

    pub async fn mark_shared(&mut self, hashes: &[String]) -> Result<()> {
        for hash in hashes {
            if let Some(candidate) = self.candidates.get_mut(hash) {
                candidate.shared = true;
            }
        }
        Ok(())
    }

    pub async fn add_versions(&mut self, versions: &[(String, ObjectVersionEntry)]) -> Result<()> {
        for (hash, version) in versions {
            check_string(hash)?;
            check_string(&version.key)?;
            check_string(&version.version_id)?;
            if serde_json::to_string(version)?.len() > MAX_ENTRY_BYTES {
                bail!("storage version metadata exceeds 16 KiB");
            }
            let id = self
                .next_id
                .checked_add(1)
                .context("media inventory version ID overflow")?;
            self.reserve::<(VersionKey, ObjectVersionEntry)>(
                hash.len() + version.key.len() + version.version_id.len(),
            )?;
            self.versions
                .insert((hash.clone(), priority(version), id), version.clone());
            self.next_id = id;
        }
        Ok(())
    }

    pub async fn add_filename(&mut self, hash: &str, name: &str) -> Result<()> {
        check_string(hash)?;
        check_string(name)?;
        let key = (hash.to_owned(), name.to_owned());
        if !self.filenames.contains(&key) {
            self.reserve::<(String, String)>(hash.len() + name.len())?;
            self.filenames.insert(key);
        }
        Ok(())
    }

    // A hash may have several stored extensions. Use the first for display;
    // every actual key remains available in the file's versions array.
    pub async fn filename(&mut self, hash: &str) -> Result<Option<String>> {
        Ok(self
            .filenames
            .range((hash.to_owned(), String::new())..)
            .next()
            .filter(|(entry_hash, _)| entry_hash == hash)
            .map(|(_, name)| name.clone()))
    }

    pub async fn require_immutable_versions(&mut self) -> Result<()> {
        for ((hash, _, _), version) in &self.versions {
            if self
                .candidates
                .get(hash)
                .is_some_and(|candidate| !candidate.shared)
                && (version.version_id.is_empty() || version.version_id == "null")
            {
                bail!("selected media has unversioned objects; immutable version IDs are required");
            }
        }
        Ok(())
    }

    pub async fn remember_marker(
        &mut self,
        prefix: &str,
        key: &str,
        version: &str,
    ) -> Result<bool> {
        for value in [prefix, key, version] {
            check_string(value)?;
        }
        let marker = (prefix.to_owned(), key.to_owned(), version.to_owned());
        if self.markers.contains(&marker) {
            return Ok(false);
        }
        self.reserve::<Marker>(prefix.len() + key.len() + version.len())?;
        self.markers.insert(marker);
        Ok(true)
    }

    pub async fn versions(
        &mut self,
        hash: &str,
        after: (i64, i64),
    ) -> Result<Vec<(i64, i64, ObjectVersionEntry)>> {
        Ok(self
            .versions
            .range((
                Excluded((hash.to_owned(), after.0, after.1)),
                Included((hash.to_owned(), i64::MAX, i64::MAX)),
            ))
            .take(usize::from(PAGE_SIZE))
            .map(|((_, priority, id), version)| (*priority, *id, version.clone()))
            .collect())
    }

    pub async fn close(self) -> Result<()> {
        Ok(())
    }
}

fn check_string(value: &str) -> Result<()> {
    if value.len() > MAX_ENTRY_BYTES {
        bail!("media inventory field exceeds 16 KiB");
    }
    Ok(())
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
#[path = "media_inventory_tests.rs"]
mod tests;
