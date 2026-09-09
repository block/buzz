use crate::error::CliError;
use buzz_core::bw::{parse_json, Consumer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(clap::Subcommand)]
pub(crate) enum BwCmd {
    /// Validate ordered signed events offline using explicit trust, evidence and now.
    Validate {
        #[arg(long)]
        input: PathBuf,
    },
    /// Display the recomputed projection of an offline history.
    Show {
        #[arg(long)]
        input: PathBuf,
    },
    /// Shape-check an unsigned draft; never sign or publish.
    DryRun {
        #[arg(long)]
        input: PathBuf,
    },
    /// Execute every named corpus case and export computed decisions (not expectations).
    Corpus {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Disabled until the full P3C readback gate is implemented.
    Publish,
}
fn input(path: &PathBuf) -> Result<Value, CliError> {
    let bytes = std::fs::read(path).map_err(|e| CliError::Usage(format!("read BW input: {e}")))?;
    parse_json(&bytes).map_err(|e| CliError::Usage(format!("strict BW JSON: {e}")))
}
fn consumer(v: &Value) -> Result<Consumer, CliError> {
    Ok(Consumer::new(
        serde_json::from_value(v["trust"].clone())
            .map_err(|e| CliError::Usage(format!("BW trust: {e}")))?,
        serde_json::from_value(v["external"].clone())
            .map_err(|e| CliError::Usage(format!("BW evidence: {e}")))?,
        v["now"]
            .as_u64()
            .filter(|n| *n <= u32::MAX as u64)
            .ok_or_else(|| CliError::Usage("explicit BW now required".into()))?,
    ))
}
fn wire(v: &Value) -> Result<Vec<u8>, CliError> {
    if let Some(raw) = v.as_str() {
        Ok(raw.as_bytes().to_vec())
    } else {
        serde_json::to_vec(v).map_err(|e| CliError::Usage(e.to_string()))
    }
}
fn evaluate(v: &Value) -> Result<(Value, Value), CliError> {
    let mut c = consumer(v)?;
    let events = v["events"]
        .as_array()
        .ok_or_else(|| CliError::Usage("BW events array required".into()))?;
    let mut results = Vec::new();
    for event in events {
        results.push(c.ingest(&wire(event)?));
    }
    Ok((json!(results), c.projection()))
}
fn corpus(v: &Value) -> Result<Value, CliError> {
    let cases = v["cases"]
        .as_array()
        .ok_or_else(|| CliError::Usage("BW cases required".into()))?;
    let mut results = Vec::new();
    for case in cases {
        let mut c = consumer(case)?;
        let mut steps = Vec::new();
        for label in case["input"]
            .as_array()
            .ok_or_else(|| CliError::Usage("case input required".into()))?
        {
            let label = label
                .as_str()
                .ok_or_else(|| CliError::Usage("event label required".into()))?;
            let event = v["events"][label]
                .get("event")
                .ok_or_else(|| CliError::Usage(format!("missing event {label}")))?;
            steps.push(c.ingest(&wire(event)?));
        }
        results.push(json!({"case":case["name"],"steps":steps}));
    }
    Ok(json!({"format":"nip-bw-results-v1","cases":results}))
}
pub(crate) fn dispatch(cmd: &BwCmd) -> Result<(), CliError> {
    let output = match cmd {
        BwCmd::Publish => {
            buzz_sdk::bw::publish().map_err(|e| CliError::Usage(e.to_string()))?;
            return Ok(());
        }
        BwCmd::DryRun { input: path } => {
            let v = input(path)?;
            let draft = buzz_sdk::bw::Draft::new(
                v["kind"]
                    .as_u64()
                    .ok_or_else(|| CliError::Usage("draft kind required".into()))?,
                v["tags"].clone(),
                v["content"]
                    .as_str()
                    .ok_or_else(|| CliError::Usage("draft content required".into()))?
                    .into(),
            )
            .map_err(|e| CliError::Usage(e.to_string()))?;
            draft
                .dry_run()
                .map_err(|e| CliError::Usage(e.to_string()))?
        }
        BwCmd::Validate { input: path } => evaluate(&input(path)?)?.0,
        BwCmd::Show { input: path } => evaluate(&input(path)?)?.1,
        BwCmd::Corpus {
            input: path,
            output,
        } => {
            let bytes = std::fs::read(path).map_err(|e| CliError::Usage(e.to_string()))?;
            let digest = hex::encode(Sha256::digest(&bytes));
            if digest != "b489fb188b45e073e81783e8d591f24891a9c73bc56d8d2fc56a881606640e8a" {
                return Err(CliError::Usage(
                    "corpus differs from pinned P1; contract review required".into(),
                ));
            }
            let data = parse_json(&bytes).map_err(|e| CliError::Usage(e.to_string()))?;
            let mut result = corpus(&data)?;
            result["p1_commit"] = json!("529136c39aa42db64af61f811fc33dac73d16dd1");
            result["fixture_sha256"] = json!(digest);
            result["document_sha256"] =
                json!("832f80c7a8b1a3119952441988369af1d53cdc1ae4c4e761ff853e68681159c3");
            let bytes =
                serde_json::to_vec_pretty(&result).map_err(|e| CliError::Other(e.to_string()))?;
            std::fs::write(output, bytes)
                .map_err(|e| CliError::Other(format!("BW export: {e}")))?;
            json!({"output":output,"cases":result["cases"].as_array().map(Vec::len),"publish_enabled":false})
        }
    };
    println!("{output}");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_json_is_independent_of_oracles() {
        let mut v: Value =
            serde_json::from_str(include_str!("../../../../docs/nips/NIP-BW.fixtures.json"))
                .expect("fixture");
        v["cases"] = json!([v["cases"][1]]);
        let first = corpus(&v).expect("execute");
        v["cases"][0]["expected"] = json!([{ "outcome":"invented" }]);
        assert_eq!(corpus(&v).expect("execute"), first);
        assert_eq!(
            first["cases"][0]["steps"][0]["event_id"],
            v["events"]["repo"]["event"]["id"]
        );
    }
    #[tokio::test]
    async fn publish_stays_disabled_without_credentials() {
        assert_eq!(crate::run_from_args(["buzz", "bw", "publish"]).await, 1);
    }
}
