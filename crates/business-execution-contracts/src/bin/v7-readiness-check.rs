#![forbid(unsafe_code)]

use business_execution_contracts::{
    ensure_v65_execution_disabled, evaluate_v7_readiness, V7Decision, V7ReadinessEvidence,
};
use std::{env, fs, process::ExitCode};

fn run() -> Result<(V7Decision, String), String> {
    ensure_v65_execution_disabled(env::var("BUSINESS_EXECUTION_ENABLED").ok().as_deref())
        .map_err(str::to_owned)?;
    let path = env::args()
        .nth(1)
        .ok_or_else(|| "usage: v7-readiness-check <evidence.json>".to_owned())?;
    let bytes = fs::read(&path).map_err(|error| format!("cannot read {path}: {error}"))?;
    let evidence: V7ReadinessEvidence = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid readiness evidence: {error}"))?;
    let report = evaluate_v7_readiness(&evidence);
    let output = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize readiness report: {error}"))?;
    Ok((report.decision, output))
}

fn main() -> ExitCode {
    match run() {
        Ok((decision, output)) => {
            println!("{output}");
            if decision == V7Decision::Ready {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            }
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}
