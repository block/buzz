//! DogStatsD lines for an acceptance [`Report`], sent over UDP.
//!
//! The format is the plain StatsD text protocol with the DogStatsD tag
//! extension (`name:value|type|#k:v,k:v`). Each line goes in its own datagram,
//! so no packet can exceed a path MTU however many tags an operator adds.

use std::net::SocketAddr;

use super::{Outcome, Report};

/// Every line for `report`: one run count and duration, then per check a run
/// count (tagged with its result and the stage that failed, or `none`), its
/// duration, and the duration of each stage it completed.
///
/// `tags` are added to every line, after `relay_version` when the relay
/// reported one. Skipped checks are counted but carry no duration.
pub fn lines(prefix: &str, report: &Report, tags: &[String]) -> Vec<String> {
    let mut common: Vec<String> = Vec::new();
    if let Some(version) = &report.relay_version {
        common.push(format!("relay_version:{}", tag_value(version)));
    }
    common.extend(tags.iter().map(|t| tag_value(t)));

    let line = |name: &str, value: String, kind: &str, own: &[String]| {
        let all: Vec<&str> = own
            .iter()
            .chain(common.iter())
            .map(String::as_str)
            .collect();
        format!("{prefix}.{name}:{value}|{kind}|#{}", all.join(","))
    };

    let run_result = if report.passed {
        Outcome::Success
    } else {
        Outcome::Failure
    };
    let mut out = vec![
        line(
            "runs",
            "1".into(),
            "c",
            &[format!("result:{}", run_result.as_str())],
        ),
        line(
            "run.seconds",
            seconds(report.seconds),
            "d",
            &[format!("result:{}", run_result.as_str())],
        ),
    ];
    for check in &report.checks {
        let result = format!("result:{}", check.result.as_str());
        let name = format!("check:{}", check.check);
        let stage = format!("stage:{}", check.failed_stage.unwrap_or("none"));
        out.push(line(
            "check.runs",
            "1".into(),
            "c",
            &[name.clone(), result.clone(), stage],
        ));
        if check.result == Outcome::Skipped {
            continue;
        }
        out.push(line(
            "check.seconds",
            seconds(check.seconds),
            "d",
            &[name.clone(), result],
        ));
        for timing in &check.stages {
            out.push(line(
                "stage.seconds",
                seconds(timing.seconds),
                "d",
                &[name.clone(), format!("stage:{}", timing.stage)],
            ));
        }
    }
    out
}

/// Sends each of `lines` to `addr` as its own UDP datagram.
///
/// Fails on the first datagram the socket refuses, so a misconfigured agent
/// address is an error for the caller rather than a run with no metrics.
pub async fn send(addr: SocketAddr, lines: &[String]) -> std::io::Result<()> {
    let bind: SocketAddr = if addr.is_ipv4() {
        ([0, 0, 0, 0], 0).into()
    } else {
        ([0u16; 8], 0).into()
    };
    let socket = tokio::net::UdpSocket::bind(bind).await?;
    for line in lines {
        socket.send_to(line.as_bytes(), addr).await?;
    }
    Ok(())
}

/// Seconds with millisecond precision; StatsD values are plain decimals.
fn seconds(s: f64) -> String {
    format!("{s:.3}")
}

/// `value` with the characters that end a DogStatsD tag or line replaced.
fn tag_value(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            ',' | '|' | '#' | '\n' | '\r' | ' ' => '_',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acceptance::{CheckReport, StageTiming};

    fn report() -> Report {
        Report {
            relay_url: "wss://relay.example".into(),
            pubkey: "00".repeat(32),
            relay_version: Some("1.2.3".into()),
            passed: false,
            seconds: 1.5,
            checks: vec![
                CheckReport {
                    check: "publish",
                    result: Outcome::Success,
                    failed_stage: None,
                    error: None,
                    seconds: 0.25,
                    stages: vec![StageTiming {
                        stage: "ok",
                        seconds: 0.25,
                    }],
                },
                CheckReport {
                    check: "fanout",
                    result: Outcome::Failure,
                    failed_stage: Some("receive"),
                    error: Some("timed out after 10s".into()),
                    seconds: 10.0,
                    stages: vec![StageTiming {
                        stage: "subscribe",
                        seconds: 0.004,
                    }],
                },
                CheckReport {
                    check: "history",
                    result: Outcome::Skipped,
                    failed_stage: None,
                    error: Some("publish failed".into()),
                    seconds: 0.0,
                    stages: vec![],
                },
            ],
        }
    }

    #[test]
    fn lines_tag_every_series_and_skip_durations_for_skipped_checks() {
        let tags = vec!["env:staging".to_string(), "target:a,b|c".to_string()];
        let got = lines("buzz.acceptance", &report(), &tags);
        let common = "relay_version:1.2.3,env:staging,target:a_b_c";
        let want = vec![
            format!("buzz.acceptance.runs:1|c|#result:failure,{common}"),
            format!("buzz.acceptance.run.seconds:1.500|d|#result:failure,{common}"),
            format!(
                "buzz.acceptance.check.runs:1|c|#check:publish,result:success,stage:none,{common}"
            ),
            format!("buzz.acceptance.check.seconds:0.250|d|#check:publish,result:success,{common}"),
            format!("buzz.acceptance.stage.seconds:0.250|d|#check:publish,stage:ok,{common}"),
            format!(
                "buzz.acceptance.check.runs:1|c|#check:fanout,result:failure,stage:receive,{common}"
            ),
            format!("buzz.acceptance.check.seconds:10.000|d|#check:fanout,result:failure,{common}"),
            format!("buzz.acceptance.stage.seconds:0.004|d|#check:fanout,stage:subscribe,{common}"),
            format!(
                "buzz.acceptance.check.runs:1|c|#check:history,result:skipped,stage:none,{common}"
            ),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn lines_without_relay_version_or_tags_still_tag_check_and_result() {
        let mut r = report();
        r.relay_version = None;
        r.passed = true;
        r.checks.truncate(1);
        let got = lines("p", &r, &[]);
        assert_eq!(got[0], "p.runs:1|c|#result:success");
        assert_eq!(
            got[2],
            "p.check.runs:1|c|#check:publish,result:success,stage:none"
        );
    }

    #[tokio::test]
    async fn send_puts_each_line_in_its_own_datagram() {
        let agent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = agent.local_addr().unwrap();
        let sent = vec!["a.b:1|c|#x:y".to_string(), "a.c:2|d|#x:y".to_string()];
        send(addr, &sent).await.unwrap();
        let mut buf = [0u8; 512];
        for want in &sent {
            let n = agent.recv(&mut buf).await.unwrap();
            assert_eq!(std::str::from_utf8(&buf[..n]).unwrap(), want);
        }
    }
}
