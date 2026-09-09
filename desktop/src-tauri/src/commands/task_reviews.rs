use serde_json::{json, Value};
use std::{fs, io::Read, path::Path};

fn repository_id(value: &str) -> Option<String> {
    let value = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("git@github.com:"))?;
    let value = value.trim_end_matches('/').trim_end_matches(".git");
    let parts: Vec<_> = value.split('/').collect();
    (parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        }))
    .then(|| value.to_ascii_lowercase())
}

fn read_reviews(
    cache: &Path,
    repository: &str,
    branch: &str,
    pr_url: Option<&str>,
) -> Result<Value, String> {
    let repository = repository_id(repository).ok_or("Expected a GitHub repository URL")?;
    let mut entries = Vec::new();
    for prefix in ["https-github.com-", "git-github.com-"] {
        let directory = cache.join(format!("{prefix}{}", repository.replace('/', "-")));
        match fs::read_dir(&directory) {
            Ok(directory) => {
                for entry in directory.take(2001) {
                    entries.push(entry.map_err(|e| e.to_string())?);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    if entries.is_empty() {
        return Ok(json!({"runs": [], "unattributed": 0, "skipped": 0}));
    }
    let mut runs = Vec::new();
    let mut skipped = 0;
    let mut unattributed = 0;
    let mut total_bytes = 0;
    let root = cache.canonicalize().map_err(|e| e.to_string())?;
    for (index, entry) in entries.into_iter().enumerate() {
        if index >= 2000 {
            return Err("Review cache exceeds 2000 runs; narrow or archive the cache".into());
        }
        let path = entry.path().join("summary.json");
        let Ok(path) = path.canonicalize() else {
            skipped += 1;
            continue;
        };
        if !path.starts_with(&root) || !path.is_file() {
            skipped += 1;
            continue;
        }
        let mut bytes = Vec::new();
        let result = fs::File::open(&path)
            .and_then(|file| file.take(1024 * 1024 + 1).read_to_end(&mut bytes));
        total_bytes += bytes.len();
        if total_bytes > 64 * 1024 * 1024 {
            return Err("Review cache exceeds the 64 MiB scan limit".into());
        }
        if result.is_err() || bytes.len() > 1024 * 1024 {
            skipped += 1;
            continue;
        }
        let Ok(mut run) = serde_json::from_slice::<Value>(&bytes) else {
            skipped += 1;
            continue;
        };
        if run["repository"]
            .as_str()
            .and_then(repository_id)
            .as_deref()
            != Some(&repository)
        {
            continue;
        }
        let metadata = &run["pr_metadata"];
        let matches = metadata["headRefName"].as_str() == Some(branch)
            || pr_url.is_some_and(|url| metadata["url"].as_str() == Some(url));
        if !matches {
            unattributed += 1;
            continue;
        }
        if !valid_run(&run) {
            skipped += 1;
            continue;
        }
        if let Some(reviews) = run["reviews"].as_array_mut() {
            for review in reviews {
                let log = review["log"]
                    .as_str()
                    .and_then(|log| Path::new(log).canonicalize().ok())
                    .filter(|log| log.starts_with(&root))
                    .map(|log| log.to_string_lossy().into_owned());
                review["log"] = json!(log);
            }
        }
        runs.push(run);
        runs.sort_by(|a, b| b["started_at"].as_str().cmp(&a["started_at"].as_str()));
        runs.truncate(20);
    }
    Ok(json!({"runs": runs, "unattributed": unattributed, "skipped": skipped}))
}

fn valid_run(run: &Value) -> bool {
    let optional_string =
        |object: &Value, key: &str| object[key].is_null() || object[key].is_string();
    (run["pr_metadata"].is_null() || run["pr_metadata"].is_object())
        && ["head", "started_at"]
            .iter()
            .all(|key| optional_string(run, key))
        && ["title", "body"]
            .iter()
            .all(|key| optional_string(&run["pr_metadata"], key))
        && run["reviews"].as_array().is_some_and(|reviews| {
            reviews.iter().all(|review| {
                review["name"].is_string()
                    && (review["verdict"].is_null() || review["verdict"].is_object())
                    && optional_string(review, "description")
                    && (review["finding_count"].is_null() || review["finding_count"].is_u64())
                    && (review["invocation_ok"].is_null() || review["invocation_ok"].is_boolean())
                    && ["overall_correctness", "overall_explanation"]
                        .iter()
                        .all(|key| optional_string(&review["verdict"], key))
                    && (review["verdict"]["findings"].is_null()
                        || review["verdict"]["findings"]
                            .as_array()
                            .is_some_and(|findings| {
                                findings.iter().all(|finding| {
                                    finding["title"].is_string() && finding["body"].is_string()
                                })
                            }))
            })
        })
}

/// Read bounded, branch-attributed local fanout summaries without exposing a file-read API.
#[tauri::command]
pub async fn get_task_reviews(
    repository: String,
    branch: String,
    pr_url: Option<String>,
) -> Result<Value, String> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs::home_dir().map(|home| home.join(".cache")))
        .ok_or("Cache directory unavailable")?
        .join("pr-review-fanout");
    tokio::task::spawn_blocking(move || {
        read_reviews(&cache, &repository, &branch, pr_url.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_review_shapes() {
        for reviews in [
            json!({}),
            json!([null]),
            json!([{"name":"x", "verdict":{"findings":{}}}]),
        ] {
            assert!(!valid_run(&json!({"reviews": reviews})));
        }
    }

    #[cfg(unix)]
    #[test]
    fn excludes_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("summary.json"), "{}").unwrap();
        let repo = temp.path().join("https-github.com-block-buzz");
        fs::create_dir_all(&repo).unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.join("escaped")).unwrap();
        let result = read_reviews(temp.path(), "https://github.com/block/buzz", "x", None).unwrap();
        assert_eq!(result["skipped"], 1);
        assert_eq!(result["runs"], json!([]));
    }

    #[test]
    #[ignore = "reads the developer's local review cache"]
    fn reads_real_review_cache() {
        let cache = dirs::home_dir().unwrap().join(".cache/pr-review-fanout");
        let result = read_reviews(
            &cache,
            "https://github.com/block/berd",
            "sol/voice-conversation-telemetry",
            Some("https://github.com/block/berd/pull/313"),
        )
        .unwrap();
        assert!(!result["runs"].as_array().unwrap().is_empty());
    }

    #[test]
    fn reads_only_matching_branch_and_bounds_files() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("https-github.com-block-buzz");
        for (name, contents) in [
            ("match", json!({"repository":"https://github.com/block/buzz.git", "pr_metadata":{"url":"https://github.com/block/buzz/pull/1"},"reviews":[]}).to_string()),
            ("other", json!({"repository":"https://github.com/block/buzz.git", "pr_metadata":{"url":"https://github.com/block/buzz/pull/2"}}).to_string()),
            ("bad", "not json".into()),
            ("big", "x".repeat(1024 * 1024 + 1)),
        ] {
            fs::create_dir_all(repo.join(name)).unwrap();
            fs::write(repo.join(name).join("summary.json"), contents).unwrap();
        }
        let result = read_reviews(
            temp.path(),
            "https://github.com/block/buzz",
            "feature",
            Some("https://github.com/block/buzz/pull/1"),
        )
        .unwrap();
        assert_eq!(result["runs"].as_array().unwrap().len(), 1);
        assert_eq!(result["unattributed"], 1);
        assert_eq!(result["skipped"], 2);
        assert!(read_reviews(temp.path(), "https://github.com/../buzz", "feature", None).is_err());
    }
}
