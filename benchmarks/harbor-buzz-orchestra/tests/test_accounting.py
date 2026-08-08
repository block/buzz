from pathlib import Path

import pytest

from harbor_buzz_orchestra import accounting
from harbor_buzz_orchestra.manifest import ExperimentManifest


def usage_line(session: str, input_tokens: int, output_tokens: int) -> str:
    """A buzz-acp usage line as tracing's compact formatter renders it."""
    return (
        "2026-07-26T01:02:03.456789Z DEBUG acp::usage: goose usage update "
        f"session_id={session} input={input_tokens} output={output_tokens}"
    )


def turn_line(label: str = "orchestrator-1", reason: str = "end_turn") -> str:
    """A buzz-acp turn-ended line (pool.rs log_stop_reason)."""
    return (
        "2026-07-26T01:02:03.456789Z  INFO pool::prompt: "
        f"turn complete for {label}: {reason}"
    )


def test_parse_usage_log_takes_the_high_water_mark_per_session():
    text = "\n".join(
        [
            "2026-07-26T01:02:03.000000Z  INFO buzz_acp: subscribed to channel c",
            usage_line("sess-1", 100, 10),
            usage_line("sess-1", 250, 40),
            usage_line("sess-2", 7, 3),
        ]
    )
    assert accounting.parse_usage_log(text) == (
        accounting.SessionUsage("sess-1", 250, 40),
        accounting.SessionUsage("sess-2", 7, 3),
    )


def test_parse_usage_log_survives_out_of_order_lines():
    """Counters are cumulative, so a late-arriving smaller value is not the total."""
    text = "\n".join([usage_line("s", 900, 90), usage_line("s", 100, 10)])
    assert accounting.parse_usage_log(text) == (accounting.SessionUsage("s", 900, 90),)


def test_parse_usage_log_reads_the_coloured_lines_a_real_trial_produces():
    """`tracing` colours its output even when redirected to a file.

    Verbatim from the first live trial (job smoke-a1-openssl). The escapes sit
    between each field name and its `=`, so an uncoloured fixture — which is
    what every other test here uses — passes while every real run parses to
    nothing and prices at zero. That failure is silent: the trial still
    succeeds, still costs money, and reports $0.00.
    """
    text = (
        "\x1b[2m2026-07-26T22:56:54.225244Z\x1b[0m \x1b[34mDEBUG\x1b[0m "
        "\x1b[2macp::usage\x1b[0m\x1b[2m:\x1b[0m goose usage update "
        "\x1b[3msession_id\x1b[0m\x1b[2m=\x1b[0mses_5a06a6a8cf2f0afb "
        "\x1b[3minput\x1b[0m\x1b[2m=\x1b[0m20658 "
        "\x1b[3moutput\x1b[0m\x1b[2m=\x1b[0m900"
    )
    assert accounting.parse_usage_log(text) == (
        accounting.SessionUsage("ses_5a06a6a8cf2f0afb", 20658, 900),
    )


def test_parse_handoffs_reads_coloured_lines_too():
    """Same formatter, same escapes — the compaction bound must not silently zero."""
    text = (
        "\x1b[2m2026-07-26T22:56:54Z\x1b[0m \x1b[32m INFO\x1b[0m "
        "\x1b[2mbuzz_agent::handoff\x1b[0m\x1b[2m:\x1b[0m "
        "handoff #1 (history 84 -> 3 items; 181430 -> 5120 tokens)"
    )
    assert accounting.parse_handoffs(text).count == 1
    assert accounting.parse_handoffs(text).input_tokens_upper_bound == 181430


def test_parse_usage_log_ignores_unrelated_output():
    text = "\n".join(
        [
            "some tool printed input=5 output=6 without the marker",
            "2026-07-26T01:02:03Z DEBUG acp::usage: goose usage update malformed",
        ]
    )
    assert accounting.parse_usage_log(text) == ()


@pytest.fixture
def solo_manifest(manifest_data) -> ExperimentManifest:
    data = dict(manifest_data)
    data["roster"] = [manifest_data["roster"][0]]
    return ExperimentManifest.model_validate(data)


def write_log(trial_dir: Path, agent_id: str, body: str) -> None:
    (trial_dir / "logs").mkdir(parents=True, exist_ok=True)
    (trial_dir / "logs" / f"{agent_id}.stdout.log").write_text(body, encoding="utf-8")


def test_collect_prices_usage_from_the_manifest(tmp_path, solo_manifest):
    write_log(tmp_path, "opus-orchestrator-1", usage_line("s", 1_000_000, 100_000))
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    assert result.input_tokens == 1_000_000
    assert result.output_tokens == 100_000
    # $5/Mtok input + $25/Mtok output
    assert result.cost_usd == pytest.approx(5.0 + 2.5)
    assert result.reconciled is True


def test_collect_flags_an_agent_that_took_turns_without_reporting_usage(
    tmp_path, solo_manifest
):
    write_log(tmp_path, "opus-orchestrator-1", turn_line())
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    assert result.cost_usd == 0.0
    # The whole point: work that burned tokens must not read as a free trial.
    assert result.reconciled is False
    assert "RUST_LOG" in result.reconciliation_note
    assert result.unmeasured_agents == ("opus-orchestrator-1",)
    assert result.idle_agents == ()


def test_collect_flags_a_trial_where_nothing_was_ever_woken(tmp_path, solo_manifest):
    write_log(tmp_path, "opus-orchestrator-1", "subscribed to channel c")
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    # Still not reconciled -- a solo agent that never took a turn means the
    # opening @mention never landed -- but the diagnosis is the harness, not the
    # usage instrumentation, and the note has to say which.
    assert result.reconciled is False
    assert "no agent was ever woken" in result.reconciliation_note
    assert "RUST_LOG" not in result.reconciliation_note
    assert result.idle_agents == ("opus-orchestrator-1",)


def test_collect_reconciles_a_roster_whose_extra_agent_never_woke(
    tmp_path, manifest_data
):
    manifest = ExperimentManifest.model_validate(manifest_data)
    orchestrator, workers = manifest.roster
    write_log(
        tmp_path,
        "opus-orchestrator-1",
        "\n".join([turn_line(), usage_line("s", 500, 50)]),
    )
    write_log(tmp_path, "qwen-workers-1", "subscribed to channel c")
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=manifest,
        agents=[
            ("opus-orchestrator-1", "orchestrator", orchestrator),
            ("qwen-workers-1", "worker", workers),
        ],
    )
    assert result.input_tokens == 500
    # An agent nobody @mentioned really did cost nothing. Failing the trial for
    # it flagged a third of the team trials for an undercount that was not there.
    assert result.reconciled is True
    assert result.idle_agents == ("qwen-workers-1",)
    assert "never woken" in result.reconciliation_note
    # An idle agent is still a row, so the roster is legible from the receipts.
    assert [receipt.agent_id for receipt in result.receipts] == [
        "opus-orchestrator-1",
        "qwen-workers-1",
    ]
    idle = result.receipts[1]
    assert idle.turns == 0
    assert idle.never_woke is True


def test_collect_fails_a_roster_whose_worker_worked_unmeasured(tmp_path, manifest_data):
    manifest = ExperimentManifest.model_validate(manifest_data)
    orchestrator, workers = manifest.roster
    write_log(
        tmp_path,
        "opus-orchestrator-1",
        "\n".join([turn_line(), usage_line("s", 500, 50)]),
    )
    write_log(tmp_path, "qwen-workers-1", turn_line("worker-1"))
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=manifest,
        agents=[
            ("opus-orchestrator-1", "orchestrator", orchestrator),
            ("qwen-workers-1", "worker", workers),
        ],
    )
    assert result.reconciled is False
    assert result.unmeasured_agents == ("qwen-workers-1",)
    assert "qwen-workers-1" in result.reconciliation_note


def test_count_turns_matches_every_stop_reason():
    text = "\n".join(
        [
            "2026-07-26T01:02:03Z  INFO pool::prompt: turn complete for a: end_turn",
            "2026-07-26T01:02:04Z  INFO pool::prompt: turn cancelled for a",
            "2026-07-26T01:02:05Z  INFO pool::prompt: turn hit max_tokens for a",
            "2026-07-26T01:02:06Z  INFO pool::prompt: turn refused for a",
            "2026-07-26T01:02:07Z  INFO buzz_acp: subscribed to channel c",
        ]
    )
    # A turn that ended badly still means the agent was woken and billed.
    assert accounting.count_turns(text) == 4
    assert accounting.count_turns("") == 0


def test_collect_reads_stderr_as_well_as_stdout(tmp_path, solo_manifest):
    (tmp_path / "logs").mkdir(parents=True)
    (tmp_path / "logs" / "opus-orchestrator-1.stderr.log").write_text(
        usage_line("s", 42, 7), encoding="utf-8"
    )
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    assert (result.input_tokens, result.output_tokens) == (42, 7)


def test_receipts_are_written_one_json_object_per_line(tmp_path, solo_manifest):
    import json

    write_log(tmp_path, "opus-orchestrator-1", usage_line("s", 10, 2))
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    path = tmp_path / "receipts.jsonl"
    result.write_receipts(path)
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    assert len(rows) == 1
    assert rows[0]["agent_id"] == "opus-orchestrator-1"
    assert rows[0]["endpoint"] == "databricks/opus"
    # The caveats travel with the number.
    assert rows[0]["cache_read_tokens_are_estimated"] is True
    assert rows[0]["reasoning_tokens_included_in_output"] is True
    assert "cost_usd_no_cache_discount" in rows[0]


def test_default_cache_rate_is_the_no_discount_upper_bound(solo_manifest):
    """Rate 0.0 must reproduce full-rate pricing exactly — no silent discount."""
    price = solo_manifest.prices["databricks/opus"]
    assert price.cache_read_rate == 0.0
    priced = accounting.price_usage(price, 1_000_000, 0)
    assert priced.cost_usd == pytest.approx(price.input_per_million_usd)
    assert priced.cost_usd == priced.cost_usd_no_cache_discount
    assert priced.estimated_cache_read_tokens == 0


def test_cache_rate_splits_input_between_the_two_rates(solo_manifest):
    price = solo_manifest.prices["databricks/opus"].model_copy(
        update={"cache_read_rate": 0.75}
    )
    priced = accounting.price_usage(price, 1_000_000, 0)
    # 250k at $5/Mtok + 750k at $0.50/Mtok
    assert priced.cost_usd == pytest.approx(1.25 + 0.375)
    assert priced.estimated_cache_read_tokens == 750_000
    # The no-discount figure is still reported, so the gap is legible.
    assert priced.cost_usd_no_cache_discount == pytest.approx(5.0)


def test_cache_rate_never_discounts_output(solo_manifest):
    """Reasoning is billed at the output rate; caching applies only to input."""
    price = solo_manifest.prices["databricks/opus"].model_copy(
        update={"cache_read_rate": 1.0}
    )
    priced = accounting.price_usage(price, 0, 1_000_000)
    assert priced.cost_usd == pytest.approx(price.output_per_million_usd)


def test_cache_rate_reaches_the_trial_totals(tmp_path, solo_manifest):
    manifest = solo_manifest.model_copy(
        update={
            "prices": {
                "databricks/opus": solo_manifest.prices["databricks/opus"].model_copy(
                    update={"cache_read_rate": 0.5}
                )
            }
        }
    )
    write_log(tmp_path, "opus-orchestrator-1", usage_line("s", 1_000_000, 0))
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=manifest,
        agents=[("opus-orchestrator-1", "orchestrator", manifest.roster[0])],
    )
    assert result.estimated_cache_read_tokens == 500_000
    assert result.cost_usd == pytest.approx(2.5 + 0.25)
    assert result.cost_usd_no_cache_discount == pytest.approx(5.0)
    assert result.assumes_cache_discount is True
    assert result.to_metadata()["accounting_assumes_cache_discount"] is True


def handoff_line(number: int, before: int, after: int = 5_120) -> str:
    """A buzz-agent auto-compaction line, as it reaches the captured stderr."""
    return (
        "2026-07-26T01:05:00.000000Z  INFO buzz_agent::handoff: "
        f"handoff #{number} (history 84 -> 3 items; {before} -> {after} tokens)"
    )


def test_parse_handoffs_bounds_the_tokens_each_compaction_burned():
    text = "\n".join([handoff_line(1, 180_000), handoff_line(2, 175_000)])
    result = accounting.parse_handoffs(text)
    assert result.count == 2
    assert result.input_tokens_upper_bound == 355_000
    assert result.output_tokens_upper_bound == 2 * accounting.HANDOFF_MAX_OUTPUT_TOKENS
    assert result.truncations == 0


def test_parse_handoffs_counts_fallbacks_to_truncation():
    """Cap reached or summary failed: context was dropped, not summarised."""
    text = "\n".join(
        [
            handoff_line(1, 180_000),
            "2026-07-26T01:06:00Z  INFO buzz_agent::handoff: "
            "handoff cap reached (10); using truncation",
            "2026-07-26T01:07:00Z  WARN buzz_agent::handoff: "
            "handoff returned empty summary; truncating",
        ]
    )
    result = accounting.parse_handoffs(text)
    assert result.count == 1
    assert result.truncations == 2


def test_parse_handoffs_ignores_unrelated_lines():
    assert accounting.parse_handoffs("handoff is a word\ntruncating output") == (
        accounting.HandoffUsage()
    )


def test_collect_keeps_the_handoff_bound_out_of_the_metered_cost(
    tmp_path, solo_manifest
):
    """cost_usd stays what the provider reported; the bound travels beside it."""
    write_log(
        tmp_path,
        "opus-orchestrator-1",
        "\n".join([usage_line("s", 1_000_000, 100_000), handoff_line(1, 180_000)]),
    )
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    assert result.input_tokens == 1_000_000
    assert result.cost_usd == pytest.approx(5.0 + 2.5)
    assert result.handoffs == 1
    # 180k input at $5/Mtok + the summary's output allowance at $25/Mtok
    expected = 180_000 * 5.0 / 1e6 + accounting.HANDOFF_MAX_OUTPUT_TOKENS * 25.0 / 1e6
    assert result.handoff_cost_usd_upper_bound == pytest.approx(expected)
    assert result.cost_usd_including_handoff_bound == pytest.approx(7.5 + expected)
    # Reconciliation still passes — the metered numbers are genuine — but the
    # note has to say the total is a floor.
    assert result.reconciled is True
    assert "floor" in result.reconciliation_note


def test_collect_warns_when_an_agent_truncated_its_own_history(
    tmp_path, solo_manifest
):
    write_log(
        tmp_path,
        "opus-orchestrator-1",
        "\n".join(
            [
                usage_line("s", 1_000, 100),
                "INFO buzz_agent::handoff: handoff cap reached (10); using truncation",
            ]
        ),
    )
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    assert result.handoff_truncations == 1
    assert any("truncated its own history" in w for w in result.warnings)


def test_a_trial_without_handoffs_reports_no_bound(tmp_path, solo_manifest):
    write_log(tmp_path, "opus-orchestrator-1", usage_line("s", 1_000, 100))
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    assert result.handoffs == 0
    assert result.handoff_cost_usd_upper_bound == 0.0
    assert result.cost_usd_including_handoff_bound == result.cost_usd
    assert "floor" not in result.reconciliation_note


# ── Measured cache reads ────────────────────────────────────────────────────
# Before these, the cache split was always modelled off the manifest's
# `cache_read_rate`, because buzz-agent read OpenAI's `prompt_tokens_details.
# cached_tokens` nowhere and buzz-acp logged only `input=`/`output=`. The input
# total was correct throughout, so cost was silently *overstated* — every cached
# token billed at the full rate — with nothing in the output to reveal it.


def usage_line_with_cache(
    session: str, input_tokens: int, output_tokens: int, cached: int
) -> str:
    """A usage line from a buzz-acp build that reports the cache split."""
    return (
        "2026-07-26T01:02:03.456789Z DEBUG acp::usage: goose usage update "
        f"session_id={session} input={input_tokens} output={output_tokens} "
        f"cached={cached}"
    )


def test_parse_usage_log_reads_the_cached_field():
    text = "\n".join(
        [
            usage_line_with_cache("s", 100, 10, 0),
            usage_line_with_cache("s", 250, 40, 90),
        ]
    )
    assert accounting.parse_usage_log(text) == (
        accounting.SessionUsage("s", 250, 40, 90),
    )


def test_parse_usage_log_without_cached_field_reports_unknown_not_zero():
    """An older agent build must not read as "nothing was cached".

    `None` routes pricing back to the modelled rate; a `0` here would assert a
    measurement the log never made and price a fully cached run at full price.
    """
    (only,) = accounting.parse_usage_log(usage_line("s", 100, 10))
    assert only.cached_input_tokens is None


def test_parse_usage_log_reads_cached_from_coloured_output():
    text = (
        "\x1b[2m2026-07-28T22:56:54.225244Z\x1b[0m \x1b[34mDEBUG\x1b[0m "
        "\x1b[2macp::usage\x1b[0m\x1b[2m:\x1b[0m goose usage update "
        "\x1b[3msession_id\x1b[0m\x1b[2m=\x1b[0mses_abc "
        "\x1b[3minput\x1b[0m\x1b[2m=\x1b[0m20658 "
        "\x1b[3moutput\x1b[0m\x1b[2m=\x1b[0m900 "
        "\x1b[3mcached\x1b[0m\x1b[2m=\x1b[0m18000"
    )
    assert accounting.parse_usage_log(text) == (
        accounting.SessionUsage("ses_abc", 20658, 900, 18000),
    )


def test_measured_cache_reads_override_the_modelled_rate(solo_manifest):
    """A real count beats the manifest's guess, in either direction."""
    price = solo_manifest.prices["databricks/opus"].model_copy(
        update={"cache_read_rate": 0.1}
    )
    priced = accounting.price_usage(
        price, 1_000_000, 0, measured_cache_read_tokens=750_000
    )
    # 250k at $5/Mtok + 750k at $0.50/Mtok -- the 0.1 rate is ignored.
    assert priced.cost_usd == pytest.approx(1.25 + 0.375)
    assert priced.estimated_cache_read_tokens == 750_000
    assert priced.cache_read_tokens_are_estimated is False


def test_a_measured_zero_prices_at_the_full_rate_but_is_not_an_estimate(solo_manifest):
    price = solo_manifest.prices["databricks/opus"].model_copy(
        update={"cache_read_rate": 0.9}
    )
    priced = accounting.price_usage(price, 1_000_000, 0, measured_cache_read_tokens=0)
    assert priced.cost_usd == pytest.approx(priced.cost_usd_no_cache_discount)
    assert priced.cache_read_tokens_are_estimated is False


def test_measured_cache_reads_are_clamped_to_the_input_total(solo_manifest):
    """Cached is a subset of input; a larger value would bill negative tokens."""
    price = solo_manifest.prices["databricks/opus"]
    priced = accounting.price_usage(price, 1_000, 0, measured_cache_read_tokens=99_999)
    assert priced.estimated_cache_read_tokens == 1_000
    assert priced.cost_usd == pytest.approx(
        1_000 * price.cached_input_per_million_usd / 1_000_000
    )


def test_collect_uses_the_measured_split_and_says_so(tmp_path, solo_manifest):
    write_log(
        tmp_path,
        "opus-orchestrator-1",
        usage_line_with_cache("s", 1_000_000, 100_000, 800_000),
    )
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    # 200k at $5 + 800k at $0.50 + 100k at $25
    assert result.cost_usd == pytest.approx(1.0 + 0.4 + 2.5)
    assert result.cost_usd_no_cache_discount == pytest.approx(5.0 + 2.5)
    assert result.estimated_cache_read_tokens == 800_000
    assert result.cache_read_tokens_are_measured is True
    assert result.to_metadata()["accounting_cache_read_tokens_are_measured"] is True


def test_collect_falls_back_to_the_model_when_a_session_is_silent_on_cache(
    tmp_path, solo_manifest
):
    """One session reporting and one not is not a trial-wide measurement.

    Summing only the sessions that spoke would under-count the cached slice and
    overstate cost, with `are_measured: True` claiming otherwise.
    """
    write_log(
        tmp_path,
        "opus-orchestrator-1",
        "\n".join(
            [
                usage_line_with_cache("s1", 1_000, 100, 900),
                usage_line("s2", 1_000, 100),
            ]
        ),
    )
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    assert result.cache_read_tokens_are_measured is False
    assert result.to_metadata()["accounting_cache_read_tokens_are_measured"] is False
    # cache_read_rate is 0.0 for this endpoint, so the fallback is the upper bound.
    assert result.cost_usd == pytest.approx(result.cost_usd_no_cache_discount)


def test_logs_without_the_cached_field_price_exactly_as_before(tmp_path, solo_manifest):
    """Back-compat: every job already on disk must produce the same number."""
    write_log(tmp_path, "opus-orchestrator-1", usage_line("s", 1_000_000, 100_000))
    result = accounting.collect(
        trial_dir=tmp_path,
        manifest=solo_manifest,
        agents=[("opus-orchestrator-1", "orchestrator", solo_manifest.roster[0])],
    )
    assert result.cost_usd == pytest.approx(5.0 + 2.5)
    assert result.cache_read_tokens_are_measured is False
