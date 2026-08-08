import assert from "node:assert/strict";
import test from "node:test";

import { CAUSAL_EXPERIMENT_SCHEMA, CausalLedger } from "./causalLedger.ts";

function experiment(id, overrides = {}) {
  return {
    schema: CAUSAL_EXPERIMENT_SCHEMA,
    experimentId: id,
    recordedAt: "2026-08-05T18:44:22Z",
    task: {
      description: "Build the Buzz Causal Ledger",
      sourceMessageId: "c5b7084",
    },
    execution: {
      sessionId: `session-${id}`,
      turnId: `turn-${id}`,
      replayOf: null,
    },
    failureFingerprint: "host-write-without-acp-permission/v1",
    context: {
      codeVersion: "2330ec7",
      policyVersion: "workspace-write/v1",
      modelVersion: "gpt-5",
      toolVersion: "buzz-acp/v1",
      environmentVersion: "macos-arm64",
    },
    hypothesis: {
      cause: "Host write bypassed ACP",
      evidenceIds: [`evidence-${id}`],
    },
    intervention: {
      remedyId: "route-through-acp/v1",
      changedVariable: "execution_path",
    },
    result: { outcome: "validated", evidenceIds: [`result-${id}`] },
    coverage: {
      acp: "observed",
      host_workspace: "observed",
      os_sandbox: "observed",
    },
    relations: { supports: [], contradicts: [], invalidates: [] },
    ...overrides,
  };
}

test("appends immutable, hash-linked causal experiments", async () => {
  const ledger = new CausalLedger();
  const first = await ledger.append(experiment("one"));
  const second = await ledger.append(experiment("two"));

  assert.equal(first.sequence, 1);
  assert.equal(second.previousHash, first.hash);
  assert.equal(await ledger.verify(), true);
  await assert.rejects(() => ledger.append(experiment("one")), /Duplicate/);
  assert.throws(() => {
    first.experiment.result.outcome = "rejected";
  }, /read only/);
});

test("restores an append-only journal and rejects a tampered restart", async () => {
  const ledger = new CausalLedger();
  await ledger.append(experiment("before-crash"));
  await ledger.append(experiment("after-restart"));
  const journal = ledger.toJournal();

  const restored = await CausalLedger.fromJournal(journal);
  assert.equal(restored.size, 2);
  assert.equal(await restored.verify(), true);

  const tampered = journal.replace("Host write bypassed ACP", "Invented cause");
  await assert.rejects(
    () => CausalLedger.fromJournal(tampered),
    /integrity failure/,
  );
});

test("does not let unrelated or version-drifted successes strengthen a finding", async () => {
  const ledger = new CausalLedger();
  const target = experiment("target");
  await ledger.append(target);
  await ledger.append(experiment("same-context"));
  await ledger.append(
    experiment("unrelated", { failureFingerprint: "different-failure/v1" }),
  );
  await ledger.append(
    experiment("drifted", {
      context: { ...target.context, policyVersion: "workspace-write/v2" },
    }),
  );

  const finding = ledger.findingFor(target);
  assert.equal(finding.comparableExperiments, 2);
  assert.equal(finding.validated, 2);
  assert.equal(finding.confidence, 1);
});

test("missing coverage and contradictions lower confidence", async () => {
  const ledger = new CausalLedger();
  const target = experiment("target");
  await ledger.append(target);
  await ledger.append(
    experiment("contradiction", {
      result: { outcome: "rejected", evidenceIds: ["contradiction-evidence"] },
      relations: { supports: [], contradicts: ["target"], invalidates: [] },
    }),
  );
  await ledger.append(
    experiment("live-dogfood-gap", {
      result: { outcome: "inconclusive", evidenceIds: [] },
      coverage: {
        acp: "observed",
        host_workspace: "missing",
        os_sandbox: "missing",
      },
    }),
  );

  const finding = ledger.findingFor(target);
  assert.deepEqual(
    {
      validated: finding.validated,
      rejected: finding.rejected,
      inconclusive: finding.inconclusive,
    },
    { validated: 1, rejected: 1, inconclusive: 1 },
  );
  assert.equal(finding.confidence, 1 / 3);
});

test("keeps 10,000 adversarial experiments inside their causal boundaries", async () => {
  const ledger = new CausalLedger();
  const target = experiment("target");
  await ledger.append(target);
  for (let index = 1; index < 10_000; index += 1) {
    const poisoned = index % 5 === 0;
    const versionDrift = index % 7 === 0;
    await ledger.append(
      experiment(`scale-${index}`, {
        failureFingerprint: poisoned
          ? "poisoned-unrelated/v1"
          : target.failureFingerprint,
        context: versionDrift
          ? { ...target.context, codeVersion: `drift-${index}` }
          : target.context,
        result: {
          outcome: index % 11 === 0 ? "rejected" : "validated",
          evidenceIds: [`result-${index}`],
        },
      }),
    );
  }

  const finding = ledger.findingFor(target);
  assert.equal(ledger.size, 10_000);
  assert.equal(await ledger.verify(), true);
  assert.equal(
    finding.comparableExperiments,
    ledger
      .entries()
      .filter(
        ({ experiment: item }) =>
          item.failureFingerprint === target.failureFingerprint &&
          item.context.codeVersion === target.context.codeVersion,
      ).length,
  );
  assert.equal(finding.experimentIds.includes("scale-5"), false);
  assert.equal(finding.experimentIds.includes("scale-7"), false);
});
