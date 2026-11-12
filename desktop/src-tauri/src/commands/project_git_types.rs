use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct ProjectRepoCommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: i64,
    pub subject: String,
}

#[derive(Serialize)]
pub struct ProjectRepoFileInfo {
    pub path: String,
    pub kind: String,
    pub size: Option<u64>,
    pub preview_content: Option<String>,
    pub last_changed_at: Option<i64>,
    pub latest_commit: Option<ProjectRepoCommitInfo>,
}

#[derive(Serialize)]
pub struct ProjectRepoContributorInfo {
    pub name: String,
    pub email: String,
    pub commit_count: usize,
    pub last_commit_at: i64,
}

#[derive(Serialize)]
pub struct ProjectRepoSnapshotInfo {
    pub latest_commit: Option<ProjectRepoCommitInfo>,
    pub commits: Vec<ProjectRepoCommitInfo>,
    pub files: Vec<ProjectRepoFileInfo>,
    pub contributors: Vec<ProjectRepoContributorInfo>,
}

#[derive(Serialize)]
pub struct ProjectLocalRepoSnapshotInfo {
    pub path: String,
    pub snapshot: ProjectRepoSnapshotInfo,
}

#[derive(Serialize)]
pub struct ProjectLocalRepoInfo {
    pub name: String,
    pub path: String,
}

#[derive(Serialize)]
pub struct ProjectRepoSyncStatusInfo {
    pub local_path: Option<String>,
    pub local_branch: Option<String>,
    pub local_branches: Vec<String>,
    pub local_head: Option<String>,
    pub local_short_head: Option<String>,
    pub remote_branch: Option<String>,
    pub remote_head: Option<String>,
    pub remote_short_head: Option<String>,
    pub merge_base: Option<String>,
    pub ahead_count: usize,
    pub behind_count: usize,
    pub has_uncommitted_changes: bool,
    pub has_untracked_files: bool,
    pub can_push: bool,
    pub push_block_reason: Option<String>,
    pub can_pull: bool,
    pub pull_block_reason: Option<String>,
}

#[derive(Serialize)]
pub struct ProjectRepoPushResult {
    pub pushed: bool,
    pub message: String,
    pub branch: String,
    pub commit: String,
    pub merge_base: Option<String>,
}

#[derive(Serialize)]
pub struct ProjectRepoPullResult {
    pub pulled: bool,
    pub message: String,
}

#[derive(Serialize)]
pub struct GitIdentityInfo {
    pub name: Option<String>,
    pub email: Option<String>,
}
