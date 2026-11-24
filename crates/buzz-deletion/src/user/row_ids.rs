//! Optional operator file of selected identifiers. A proposal can precede a failed write.
use buzz_db::{entity_deletion::EntityStage, user_deletion::ExecutionMode};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
};

pub(super) fn create(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub(super) struct RowIds<'a> {
    file: &'a File,
    stage: &'a EntityStage,
    mode: ExecutionMode,
}

impl<'a> RowIds<'a> {
    pub fn new(file: &'a File, stage: &'a EntityStage, mode: ExecutionMode) -> Self {
        Self { file, stage, mode }
    }

    pub fn record(&self, mut entry: Value) -> io::Result<()> {
        entry["request_id"] = json!(self.stage.request_id);
        entry["stage"] = json!(self.stage.stage);
        entry["stage_id"] = json!(self.stage.id);
        entry["community"] = json!(self.stage.target_community_id);
        entry["mode"] = json!(if self.mode == ExecutionMode::Delete {
            "delete"
        } else {
            "read-only"
        });
        let mut bytes = serde_json::to_vec(&entry)?;
        bytes.push(b'\n');
        let mut file = self.file;
        file.write_all(&bytes)
    }
}

/// The three database planners already stream bounded batches of JSON lines.
/// Keep just the lookup key and proposed action, dropping reasons and summaries.
pub(super) struct BatchOutput<'a> {
    ids: RowIds<'a>,
    line: Vec<u8>,
}

impl<'a> BatchOutput<'a> {
    pub fn new(ids: RowIds<'a>) -> Self {
        Self {
            ids,
            line: Vec::new(),
        }
    }

    fn batch(&self) -> io::Result<()> {
        let batch: Value = serde_json::from_slice(&self.line)?;
        if !matches!(batch["record"].as_str(), Some("proposed_batch" | "batch")) {
            return Ok(());
        }
        if let Some(keys) = batch["keys"].as_array() {
            for key in keys {
                self.ids
                    .record(json!({"table":batch["table"],"action":batch["action"],"key":key}))?;
            }
        } else {
            let items = batch["items"]
                .as_array()
                .ok_or_else(|| io::Error::other("planned batch is missing its row identifiers"))?;
            for item in items {
                self.ids.record(
                    json!({"table":item["table"],"action":item["action"],"key":item["key"]}),
                )?;
            }
        }
        Ok(())
    }
}

impl Write for BatchOutput<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for chunk in buf.split_inclusive(|b| *b == b'\n') {
            self.line.extend_from_slice(chunk);
            if chunk.ends_with(b"\n") {
                io::stderr().write_all(&self.line)?;
                self.batch()?;
                self.line.clear();
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl super::worker::RunOptions {
    pub(super) fn mode(&self) -> ExecutionMode {
        if self.delete {
            ExecutionMode::Delete
        } else {
            ExecutionMode::ReadOnly
        }
    }

    pub(super) fn ids<'a>(&'a self, stage: &'a EntityStage) -> Option<RowIds<'a>> {
        self.row_ids
            .as_ref()
            .map(|f| RowIds::new(f, stage, self.mode()))
    }

    pub(super) fn batch_output<'a>(&'a self, stage: &'a EntityStage) -> Box<dyn Write + Send + 'a> {
        match self.ids(stage) {
            Some(ids) => Box::new(BatchOutput::new(ids)),
            None => Box::new(io::stderr()),
        }
    }
}
