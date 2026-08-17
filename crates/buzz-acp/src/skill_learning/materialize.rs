use std::{
    collections::HashSet,
    fs::{self, File},
    io::Write,
    path::Path,
};

use buzz_core::agent_skill::{skill_body_hash, SkillVersionV1};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MARKER_FILE: &str = ".skill-version.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedSkillMarkerV1 {
    pub schema_version: u8,
    pub skill_id: String,
    pub version_id: String,
    pub content_hash: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MaterializeReport {
    pub installed: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub preserved_unverified: usize,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum MaterializeError {
    #[error("active skill is invalid")]
    InvalidSkill,
    #[error("managed skill set contains a duplicate identity")]
    DuplicateSkill,
    #[error("existing managed-looking directory is not verifiable")]
    UnverifiedTarget,
    #[error("managed skill serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("managed skill filesystem operation failed")]
    Io(#[from] std::io::Error),
}

pub(crate) fn materialize_active_skills(
    root: &Path,
    active: &[SkillVersionV1],
) -> Result<MaterializeReport, MaterializeError> {
    let mut active_ids = HashSet::new();
    for version in active {
        version
            .validate()
            .map_err(|_| MaterializeError::InvalidSkill)?;
        if !active_ids.insert(version.skill_id.clone()) {
            return Err(MaterializeError::DuplicateSkill);
        }
    }

    fs::create_dir_all(root)?;
    let mut report = MaterializeReport::default();
    for version in active {
        let target = root.join(&version.skill_id);
        if target.exists() {
            let Some(marker) = read_verified_marker(&target) else {
                return Err(MaterializeError::UnverifiedTarget);
            };
            if marker.version_id == version.version_id
                && marker.content_hash == version.content_hash
                && fs::read_to_string(target.join("SKILL.md")).ok().as_deref()
                    == Some(version.skill_md.as_str())
            {
                report.unchanged += 1;
                continue;
            }
        }
        install_version(root, version)?;
        report.installed += 1;
    }

    for entry in fs::read_dir(root)?.filter_map(Result::ok) {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_managed_skill_id(name) || active_ids.contains(name) || !path.is_dir() {
            continue;
        }
        if read_verified_marker(&path).is_some() {
            fs::remove_dir_all(&path)?;
            report.removed += 1;
        } else {
            report.preserved_unverified += 1;
        }
    }
    sync_dir(root)?;
    Ok(report)
}

fn install_version(root: &Path, version: &SkillVersionV1) -> Result<(), MaterializeError> {
    let target = root.join(&version.skill_id);
    let stage = root.join(format!(".{}-stage-{}", version.skill_id, Uuid::new_v4()));
    let backup = root.join(format!(".{}-backup-{}", version.skill_id, Uuid::new_v4()));
    fs::create_dir(&stage)?;
    let marker = ManagedSkillMarkerV1 {
        schema_version: 1,
        skill_id: version.skill_id.clone(),
        version_id: version.version_id.clone(),
        content_hash: version.content_hash.clone(),
    };
    write_synced(&stage.join("SKILL.md"), version.skill_md.as_bytes())?;
    let marker_bytes = serde_json::to_vec_pretty(&marker)?;
    write_synced(&stage.join(MARKER_FILE), &marker_bytes)?;
    sync_dir(&stage)?;
    let staged_body = fs::read_to_string(stage.join("SKILL.md"))?;
    if skill_body_hash(&staged_body) != version.content_hash {
        let _ = fs::remove_dir_all(&stage);
        return Err(MaterializeError::InvalidSkill);
    }

    let had_target = target.exists();
    if had_target {
        fs::rename(&target, &backup)?;
    }
    if let Err(error) = fs::rename(&stage, &target) {
        if had_target {
            let _ = fs::rename(&backup, &target);
        }
        let _ = fs::remove_dir_all(&stage);
        return Err(MaterializeError::Io(error));
    }
    sync_dir(root)?;
    if had_target {
        fs::remove_dir_all(&backup)?;
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn sync_dir(path: &Path) -> Result<(), std::io::Error> {
    File::open(path)?.sync_all()
}

pub(crate) fn read_verified_marker(directory: &Path) -> Option<ManagedSkillMarkerV1> {
    let name = directory.file_name()?.to_str()?;
    if !is_managed_skill_id(name) {
        return None;
    }
    let marker: ManagedSkillMarkerV1 =
        serde_json::from_slice(&fs::read(directory.join(MARKER_FILE)).ok()?).ok()?;
    let body = fs::read_to_string(directory.join("SKILL.md")).ok()?;
    if marker.schema_version != 1
        || marker.skill_id != name
        || !is_safe_id(&marker.version_id)
        || marker.content_hash != skill_body_hash(&body)
    {
        return None;
    }
    Some(marker)
}

fn is_managed_skill_id(value: &str) -> bool {
    value.strip_prefix("learned-").is_some_and(|suffix| {
        suffix.len() == 12
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}
