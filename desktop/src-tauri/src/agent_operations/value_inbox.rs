use std::{collections::BTreeMap, fs, path::Path};

use chrono::NaiveDate;
use regex::Regex;

use super::calendar::monday_for;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    Apply,
    Retain,
    Discard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValueInboxSummary {
    pub videos: usize,
    pub applied: usize,
    pub retained: usize,
    pub incomplete: bool,
}

impl ValueInboxSummary {
    pub(crate) fn line(&self) -> String {
        let prefix = if self.incomplete {
            "YouTube Value Inbox: Incomplete; known "
        } else {
            "YouTube Value Inbox: "
        };
        format!(
            "{prefix}videos this week {}, applied {}, retained {}",
            self.videos, self.applied, self.retained
        )
    }
}

fn parse_created(contents: &str) -> Option<NaiveDate> {
    let frontmatter = contents.strip_prefix("---\n")?.split_once("\n---")?.0;
    let value = frontmatter.lines().find_map(|line| {
        line.strip_prefix("created:")
            .map(str::trim)
            .map(|value| value.trim_matches(['\'', '"']))
    })?;
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

fn parse_metadata(contents: &str, filename_video_id: &str) -> Option<Disposition> {
    let fence = Regex::new(r"(?s)```json\s*(\{.*?\})\s*```").expect("static regex");
    let mut candidates = fence.captures_iter(contents).filter_map(|capture| {
        let value: serde_json::Value = serde_json::from_str(capture.get(1)?.as_str()).ok()?;
        (value.get("source").is_some() || value.get("disposition").is_some()).then_some(value)
    });
    let value = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    if value.pointer("/source/video_id")?.as_str()? != filename_video_id {
        return None;
    }
    match value.get("disposition")?.as_str()? {
        "Apply" => Some(Disposition::Apply),
        "Retain" => Some(Disposition::Retain),
        "Discard" => Some(Disposition::Discard),
        _ => None,
    }
}

pub(crate) fn scan_value_inbox(research_dir: &Path, digest_date: NaiveDate) -> ValueInboxSummary {
    let name_pattern = Regex::new(r"^VIDEO_([A-Za-z0-9_-]{11})_.+\.md$").expect("static regex");
    let monday = monday_for(digest_date);
    let mut incomplete = false;
    let mut dispositions: BTreeMap<String, Vec<Disposition>> = BTreeMap::new();

    let entries = match fs::read_dir(research_dir) {
        Ok(entries) => entries,
        Err(_) => {
            return ValueInboxSummary {
                videos: 0,
                applied: 0,
                retained: 0,
                incomplete: true,
            };
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                incomplete = true;
                continue;
            }
        };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(captures) = name_pattern.captures(&name) else {
            continue;
        };
        let video_id = captures[1].to_string();
        let contents = match fs::read_to_string(entry.path()) {
            Ok(contents) => contents,
            Err(_) => {
                incomplete = true;
                continue;
            }
        };
        let Some(created) = parse_created(&contents) else {
            incomplete = true;
            continue;
        };
        if created > digest_date {
            continue;
        }
        if created < monday {
            continue;
        }
        let Some(disposition) = parse_metadata(&contents, &video_id) else {
            incomplete = true;
            continue;
        };
        dispositions.entry(video_id).or_default().push(disposition);
    }

    let mut videos = 0;
    let mut applied = 0;
    let mut retained = 0;
    for values in dispositions.values() {
        let first = values[0];
        if values.iter().any(|value| *value != first) {
            incomplete = true;
            videos += 1;
            continue;
        }
        videos += 1;
        match first {
            Disposition::Apply => applied += 1,
            Disposition::Retain => retained += 1,
            Disposition::Discard => {}
        }
    }
    ValueInboxSummary {
        videos,
        applied,
        retained,
        incomplete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn brief(created: &str, id: &str, disposition: &str, surrounding: &str) -> String {
        format!(
            "---\ntitle: \"Test\"\ncreated: {created}\n---\n{surrounding}\n```json\n{{\"source\":{{\"video_id\":\"{id}\",\"publication_date\":\"1999-01-01\"}},\"disposition\":\"{disposition}\"}}\n```\n"
        )
    }

    #[test]
    fn syn79_value_inbox_week_to_date_complete_counts_are_exact() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("VIDEO_aaaaaaaaaaa_apply.md"),
            brief("2026-09-01", "aaaaaaaaaaa", "Apply", "@hostile | text"),
        )
        .unwrap();
        fs::write(
            dir.path().join("VIDEO_bbbbbbbbbbb_retain.md"),
            brief("2026-09-02", "bbbbbbbbbbb", "Retain", "```oops```"),
        )
        .unwrap();
        fs::write(
            dir.path().join("VIDEO_ccccccccccc_discard.md"),
            brief("2026-09-02", "ccccccccccc", "Discard", ""),
        )
        .unwrap();
        fs::write(
            dir.path().join("VIDEO_ddddddddddd_old.md"),
            brief("2026-08-30", "ddddddddddd", "Apply", ""),
        )
        .unwrap();
        fs::write(
            dir.path().join("VIDEO_eeeeeeeeeee_future.md"),
            brief("2026-09-03", "eeeeeeeeeee", "Apply", ""),
        )
        .unwrap();
        let summary = scan_value_inbox(dir.path(), NaiveDate::from_ymd_opt(2026, 9, 2).unwrap());
        assert_eq!(
            summary.line(),
            "YouTube Value Inbox: videos this week 3, applied 1, retained 1"
        );
    }

    #[test]
    fn syn79_value_inbox_invalid_and_conflicting_briefs_use_known_lower_bounds() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("VIDEO_aaaaaaaaaaa_one.md"),
            brief("2026-09-01", "aaaaaaaaaaa", "Apply", ""),
        )
        .unwrap();
        fs::write(
            dir.path().join("VIDEO_aaaaaaaaaaa_two.md"),
            brief("2026-09-02", "aaaaaaaaaaa", "Retain", ""),
        )
        .unwrap();
        fs::write(
            dir.path().join("VIDEO_bbbbbbbbbbb_bad.md"),
            brief("2026-09-02", "wrong-id00", "Apply", ""),
        )
        .unwrap();
        fs::write(
            dir.path().join("VIDEO_ccccccccccc_case.md"),
            brief("2026-09-02", "ccccccccccc", "apply", ""),
        )
        .unwrap();
        fs::write(
            dir.path().join("VIDEO_ddddddddddd_missing_date.md"),
            "---\ntitle: \"Missing date\"\n---\n```json\n{\"source\":{\"video_id\":\"ddddddddddd\"},\"disposition\":\"Retain\"}\n```\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("VIDEO_eeeeeeeeeee_invalid_date.md"),
            brief("not-a-date", "eeeeeeeeeee", "Retain", ""),
        )
        .unwrap();
        fs::write(
            dir.path().join("VIDEO_fffffffffff_bad_json.md"),
            "---\ncreated: 2026-09-02\n---\n```json\n{broken}\n```\n",
        )
        .unwrap();
        fs::create_dir(dir.path().join("VIDEO_ggggggggggg_unreadable.md")).unwrap();
        let summary = scan_value_inbox(dir.path(), NaiveDate::from_ymd_opt(2026, 9, 2).unwrap());
        assert_eq!(
            summary.line(),
            "YouTube Value Inbox: Incomplete; known videos this week 1, applied 0, retained 0"
        );
    }

    #[test]
    fn syn79_value_inbox_matching_duplicates_count_once_and_empty_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, 2).unwrap();
        assert_eq!(
            scan_value_inbox(dir.path(), date).line(),
            "YouTube Value Inbox: videos this week 0, applied 0, retained 0"
        );
        for suffix in ["one", "two"] {
            fs::write(
                dir.path().join(format!("VIDEO_aaaaaaaaaaa_{suffix}.md")),
                brief("2026-09-02", "aaaaaaaaaaa", "Retain", ""),
            )
            .unwrap();
        }
        assert_eq!(scan_value_inbox(dir.path(), date).videos, 1);
    }
}
