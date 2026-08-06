import { readFileSync } from "node:fs";
import { isAbsolute } from "node:path";

import type { Page } from "@playwright/test";

import type { CurrentProjection } from "../../../src/features/binding-status/currentProjectionStore";

const TRACE_ENV = "BUZZ_J3C_PROJECTION_TRACE";
const LOWERCASE_HEX_256 = /^[0-9a-f]{64}$/;
const CANONICAL_UUID_V4 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

export const CURRENT_BINDING_TRACE_CASES = [
  "bootstrap",
  "current",
  "duplicate",
  "equal-conflict",
  "rollback",
  "newer-restoration",
  "withdrawal",
  "passive-expiry",
  "disconnect",
  "reconnect",
  "logout",
  "restart",
  "relay-scope-change",
  "signer-scope-change",
  "author-scope-change",
  "domain-scope-change",
  "epoch-scope-change",
  "malformed-trusted",
  "unsupported-version",
  "author-mismatch",
  "profile-spoof",
  "nip85-no-fallback",
] as const;

const CASES_WITH_CURRENT_PROJECTION = new Set<CurrentBindingTraceCase>([
  "current",
  "duplicate",
  "newer-restoration",
  "reconnect",
]);

export type CurrentBindingTraceCase =
  (typeof CURRENT_BINDING_TRACE_CASES)[number];

export type NativeCurrentProjection = CurrentProjection;

export type CurrentBindingTraceStep = Readonly<{
  case: CurrentBindingTraceCase;
  projection: NativeCurrentProjection | null;
}>;

export type CurrentBindingStatusTrace = Readonly<{
  version: 1;
  steps: readonly CurrentBindingTraceStep[];
}>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasExactKeys(value: Record<string, unknown>, expected: string[]) {
  const actual = Object.keys(value).sort();
  const sortedExpected = [...expected].sort();
  return (
    actual.length === sortedExpected.length &&
    actual.every((key, index) => key === sortedExpected[index])
  );
}

function isCurrentProjection(value: unknown): value is NativeCurrentProjection {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["connectionEpoch", "eventAuthorPubkey", "freshUntil"])
  ) {
    return false;
  }

  return (
    typeof value.eventAuthorPubkey === "string" &&
    LOWERCASE_HEX_256.test(value.eventAuthorPubkey) &&
    typeof value.freshUntil === "number" &&
    Number.isSafeInteger(value.freshUntil) &&
    value.freshUntil > 0 &&
    typeof value.connectionEpoch === "string" &&
    CANONICAL_UUID_V4.test(value.connectionEpoch)
  );
}

function parseTrace(value: unknown, path: string): CurrentBindingStatusTrace {
  if (!isRecord(value) || !hasExactKeys(value, ["steps", "version"])) {
    throw new Error(`${path} is not an exact J3C projection trace object.`);
  }
  if (value.version !== 1 || !Array.isArray(value.steps)) {
    throw new Error(`${path} must contain trace version 1 and a steps array.`);
  }
  if (value.steps.length !== CURRENT_BINDING_TRACE_CASES.length) {
    throw new Error(
      `${path} must contain exactly ${CURRENT_BINDING_TRACE_CASES.length} trace steps.`,
    );
  }

  for (const [index, expectedCase] of CURRENT_BINDING_TRACE_CASES.entries()) {
    const step = value.steps[index];
    if (!isRecord(step) || !hasExactKeys(step, ["case", "projection"])) {
      throw new Error(`${path} step ${index} is not an exact trace step.`);
    }
    if (step.case !== expectedCase) {
      throw new Error(
        `${path} step ${index} must be case ${expectedCase}, received ${String(step.case)}.`,
      );
    }

    const expectsCurrent = CASES_WITH_CURRENT_PROJECTION.has(expectedCase);
    if (
      (expectsCurrent && !isCurrentProjection(step.projection)) ||
      (!expectsCurrent && step.projection !== null)
    ) {
      throw new Error(
        `${path} case ${expectedCase} has an invalid retained projection.`,
      );
    }
  }

  // Return the parsed objects themselves. The Playwright boundary forwards the
  // native DTO without rebuilding, enriching, or substituting it.
  return value as CurrentBindingStatusTrace;
}

export function loadCurrentBindingStatusTrace(): CurrentBindingStatusTrace {
  const path = process.env[TRACE_ENV];
  if (!path) {
    throw new Error(
      `${TRACE_ENV} is required and must name the Rust native-flow trace.`,
    );
  }
  if (!isAbsolute(path)) {
    throw new Error(`${TRACE_ENV} must be an absolute path; received ${path}.`);
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    const reason = error instanceof Error ? error.message : "unknown error";
    throw new Error(`Unable to read ${TRACE_ENV} at ${path}: ${reason}.`);
  }
  return parseTrace(parsed, path);
}

export function traceStep(
  trace: CurrentBindingStatusTrace,
  caseName: CurrentBindingTraceCase,
): CurrentBindingTraceStep {
  const step = trace.steps.find((candidate) => candidate.case === caseName);
  if (!step) {
    throw new Error(`Native projection trace is missing case ${caseName}.`);
  }
  return step;
}

export async function installNativeProjectionTraceAdapter(
  page: Page,
): Promise<void> {
  await page.addInitScript(() => {
    type Invoke = (
      command: string,
      args: Record<string, unknown>,
      options: unknown,
    ) => unknown;
    type ProjectionChannel = {
      onmessage: (projection: unknown) => void;
    };
    type TraceAdapterWindow = typeof window & {
      __TAURI_INTERNALS__?: Record<string, unknown>;
      __BUZZ_J3C_FORWARD_NATIVE_PROJECTION__?: (projection: unknown) => void;
      __BUZZ_J3C_STATUS_AUTH_BOUND__?: () => boolean;
    };

    const tauriWindow = window as TraceAdapterWindow;
    const internals = tauriWindow.__TAURI_INTERNALS__ ?? {};
    let sharedInvoke =
      typeof internals.invoke === "function"
        ? (internals.invoke as Invoke)
        : null;
    let projectionChannel: ProjectionChannel | null = null;
    let statusSocketId: number | null = null;
    let authBound = false;

    const invoke: Invoke = (command, args, options) => {
      if (!sharedInvoke) {
        throw new Error(`Shared mock bridge is not installed for ${command}.`);
      }

      if (command === "plugin:websocket|connect_with_status") {
        const { onProjection, ...ordinaryArgs } = args as {
          onProjection?: ProjectionChannel;
        } & Record<string, unknown>;
        if (typeof onProjection?.onmessage !== "function") {
          throw new Error(
            "Status connection omitted its native projection Channel.",
          );
        }

        authBound = false;
        projectionChannel = onProjection;
        statusSocketId = null;
        return Promise.resolve(
          sharedInvoke("plugin:websocket|connect", ordinaryArgs, options),
        ).then((id) => {
          if (typeof id !== "number" || !Number.isSafeInteger(id)) {
            throw new Error(
              "Shared mock bridge returned an invalid socket ID.",
            );
          }
          statusSocketId = id;
          return id;
        });
      }

      if (command === "create_auth_event") {
        if (
          statusSocketId === null ||
          args.nativeWebsocketId !== statusSocketId
        ) {
          throw new Error(
            "Auth event is not bound to the current native status socket ID.",
          );
        }
        authBound = true;
      }

      return sharedInvoke(command, args, options);
    };

    Object.defineProperty(internals, "invoke", {
      configurable: true,
      get: () => invoke,
      set: (nextInvoke: Invoke) => {
        sharedInvoke = nextInvoke;
      },
    });
    tauriWindow.__TAURI_INTERNALS__ = internals;
    tauriWindow.__BUZZ_J3C_STATUS_AUTH_BOUND__ = () => authBound;
    tauriWindow.__BUZZ_J3C_FORWARD_NATIVE_PROJECTION__ = (projection) => {
      if (!authBound || statusSocketId === null || !projectionChannel) {
        throw new Error(
          "Native status projection Channel is not authenticated.",
        );
      }
      projectionChannel.onmessage(projection);
    };
  });
}

export async function waitForNativeProjectionTraceAdapter(
  page: Page,
): Promise<void> {
  await page.waitForFunction(
    () =>
      (
        window as typeof window & {
          __BUZZ_J3C_STATUS_AUTH_BOUND__?: () => boolean;
        }
      ).__BUZZ_J3C_STATUS_AUTH_BOUND__?.() === true,
  );
}

export async function forwardTraceStep(
  page: Page,
  step: CurrentBindingTraceStep,
): Promise<void> {
  await page.evaluate((projection) => {
    const forward = (
      window as typeof window & {
        __BUZZ_J3C_FORWARD_NATIVE_PROJECTION__?: (projection: unknown) => void;
      }
    ).__BUZZ_J3C_FORWARD_NATIVE_PROJECTION__;
    if (!forward)
      throw new Error("Native projection adapter is not installed.");
    forward(projection);
  }, step.projection);
}
