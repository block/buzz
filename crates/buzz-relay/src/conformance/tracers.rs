//! Concrete [`Tracer`] implementations. Production uses [`NoopTracer`];
//! conformance tests + the CI replay job use [`JsonlTracer`].

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use buzz_conformance::{TraceStep, Tracer};

/// Zero-cost tracer used in production builds. Records nothing — the
/// emitter call still constructs the action arguments, but the build can
/// have the compiler eliminate them entirely behind a feature flag if
/// the cost ever shows up in benches.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTracer;

impl Tracer for NoopTracer {
    fn record(&self, _step: TraceStep) {}
}

/// JSONL-to-file tracer for tests + the CI replay job. Each `record` call
/// serializes the step as one line of JSON and appends it. The file is
/// opened in append mode so multiple test runs accumulate; consumers are
/// expected to truncate between runs.
///
/// The internal `Mutex<BufWriter<File>>` serializes writes — concurrent
/// requests producing interleaved JSONL is fine on the read side because
/// the spec doesn't model emission order, only set membership.
pub struct JsonlTracer {
    out: Mutex<BufWriter<File>>,
}

impl JsonlTracer {
    /// Open a new JSONL tracer writing to `path`. Truncates any existing
    /// file at that path so a fresh test run starts clean.
    pub fn create<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(Self {
            out: Mutex::new(BufWriter::new(file)),
        })
    }
}

impl std::fmt::Debug for JsonlTracer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonlTracer").finish_non_exhaustive()
    }
}

impl Tracer for JsonlTracer {
    fn record(&self, step: TraceStep) {
        // Acquire-and-write. If the lock is poisoned we accept the panic
        // — this is observability code and a poisoned lock means a worse
        // bug landed elsewhere.
        let mut guard = match self.out.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        // Best-effort: a write failure here loses one trace step but
        // must NOT take down the request path. The Drop guard's
        // coverage-breach action is the safety net for systemic loss.
        if let Ok(line) = serde_json::to_string(&step) {
            let _ = guard.write_all(line.as_bytes());
            let _ = guard.write_all(b"\n");
            let _ = guard.flush();
        }
    }
}
