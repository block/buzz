import { appendFile, readFile } from "node:fs/promises";

// Playwright's `retries: 2` (desktop/playwright.config.ts) lets a test fail
// then pass on retry with no durable signal beyond a one-line "N flaky" in
// the console log — the exact gap that hid the stream.spec.ts membership
// race (#1798) for months. This walks the JSON reporter's suite tree
// (recursive: `describe` blocks nest as child `suites`) and appends any
// `status === "flaky"` test to the job's GitHub Actions summary so retried
// failures stay visible even when the shard ultimately goes green.
//
// Usage: node scripts/summarize-flaky-tests.mjs <report.json> <run-label>

function collectFlakyTests(suite, out) {
  for (const spec of suite.specs ?? []) {
    for (const test of spec.tests ?? []) {
      if (test.status !== "flaky") continue;
      out.push({
        title: `${suite.file} › ${spec.title}`,
        project: test.projectName,
        attempts: test.results?.length ?? 0,
      });
    }
  }
  for (const child of suite.suites ?? []) {
    collectFlakyTests(child, out);
  }
}

const [reportPath, runLabel] = process.argv.slice(2);
if (!reportPath || !runLabel) {
  console.error(
    "Usage: node scripts/summarize-flaky-tests.mjs <report.json> <run-label>",
  );
  process.exit(1);
}

// This step runs `if: !cancelled()` purely to surface flaky tests — it must
// never fail the job on its own, so a malformed/unexpected report (from a
// Playwright version bump or a crashed run) is swallowed, not thrown.
try {
  const report = JSON.parse(await readFile(reportPath, "utf8"));

  const flaky = [];
  for (const suite of report.suites ?? []) {
    collectFlakyTests(suite, flaky);
  }

  if (flaky.length > 0) {
    const escapeCell = (value) => String(value).replaceAll("|", "\\|");
    const rows = flaky
      .map(
        (t) =>
          `| ${escapeCell(t.title)} | ${escapeCell(t.project)} | ${t.attempts} |`,
      )
      .join("\n");
    const summary =
      `### Flaky tests — ${runLabel}\n\n` +
      `${flaky.length} test(s) failed at least once before passing on retry:\n\n` +
      "| Test | Project | Attempts |\n| --- | --- | --- |\n" +
      `${rows}\n`;

    console.log(summary);

    const summaryFile = process.env.GITHUB_STEP_SUMMARY;
    if (summaryFile) {
      await appendFile(summaryFile, `${summary}\n`);
    }
  }
} catch (error) {
  console.log(`Skipping flaky-test summary: ${error.message}`);
}
