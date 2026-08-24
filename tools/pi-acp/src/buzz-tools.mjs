import crypto from "node:crypto";
import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { defineTool } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const HEX_EVENT = /^[0-9a-f]{64}$/i;
const UUID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const MAX_CONTENT_BYTES = 64 * 1024;
const MAX_OUTPUT_BYTES = 64 * 1024;

function validateContext(context) {
  if (!context || !UUID.test(context.channelId || "")) {
    throw new Error("authenticated Buzz channel context is unavailable");
  }
  if (!HEX_EVENT.test(context.replyTo || "")) {
    throw new Error("authenticated Buzz reply target is unavailable");
  }
  if (
    !Array.isArray(context.triggeringEventIds) ||
    context.triggeringEventIds.length === 0 ||
    !context.triggeringEventIds.every((eventId) => HEX_EVENT.test(eventId)) ||
    !context.triggeringEventIds.includes(context.replyTo)
  ) {
    throw new Error("authenticated Buzz triggering event set is invalid");
  }
  return context;
}

function toolText(text, details = {}) {
  return { content: [{ type: "text", text }], details };
}

async function run(
  command,
  args,
  { input, signal, env = process.env, timeoutMs = 30_000 } = {},
) {
  return await new Promise((resolve, reject) => {
    let timer;
    const child = spawn(command, args, {
      env,
      stdio: [input === undefined ? "ignore" : "pipe", "pipe", "pipe"],
    });
    let stdout = Buffer.alloc(0);
    let stderr = Buffer.alloc(0);
    let settled = false;
    const finish = (error, result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
      if (error) reject(error);
      else resolve(result);
    };
    const append = (current, chunk) => {
      const next = Buffer.concat([current, chunk]);
      if (next.length > MAX_OUTPUT_BYTES) {
        child.kill("SIGKILL");
        finish(new Error(`tool output exceeds ${MAX_OUTPUT_BYTES} bytes`));
      }
      return next;
    };
    child.stdout.on("data", (chunk) => {
      stdout = append(stdout, chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderr = append(stderr, chunk);
    });
    child.on("error", (error) => finish(error));
    child.on("exit", (code, exitSignal) => {
      const result = {
        code,
        signal: exitSignal,
        stdout: stdout.toString("utf8").trim(),
        stderr: stderr.toString("utf8").trim(),
      };
      if (code === 0) finish(undefined, result);
      else
        finish(
          new Error(result.stderr || `${command} exited with code ${code}`),
        );
    });
    const abort = () => child.kill("SIGTERM");
    signal?.addEventListener("abort", abort, { once: true });
    timer = setTimeout(() => {
      child.kill("SIGKILL");
      finish(new Error(`${command} timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    if (input !== undefined) child.stdin.end(input);
  });
}

function reservationKey(context, env) {
  const secret = env.BUZZ_PRIVATE_KEY;
  if (!secret) throw new Error("BUZZ_PRIVATE_KEY is unavailable");
  const material = JSON.stringify({
    relay: env.BUZZ_RELAY_URL || "",
    channelId: context.channelId,
    triggeringEventIds: [...context.triggeringEventIds].sort(),
    replyTo: context.replyTo,
  });
  return crypto.createHmac("sha256", secret).update(material).digest("hex");
}

async function reservePublication(context, content, env) {
  const contentSha256 = crypto
    .createHash("sha256")
    .update(content)
    .digest("hex");
  const root =
    env.PI_ACP_RECEIPT_DIR ||
    path.join(os.homedir(), ".buzz", "pi-acp-receipts");
  await fs.mkdir(root, { recursive: true, mode: 0o700 });
  const directory = path.join(root, reservationKey(context, env));
  try {
    await fs.mkdir(directory, { mode: 0o700 });
    await fs.writeFile(
      path.join(directory, "request.json"),
      `${JSON.stringify({ ...context, contentSha256 })}\n`,
      { mode: 0o600 },
    );
    return { directory, existingReceipt: null };
  } catch (error) {
    if (error.code !== "EEXIST") throw error;
    try {
      const request = JSON.parse(
        await fs.readFile(path.join(directory, "request.json"), "utf8"),
      );
      if (request.contentSha256 !== contentSha256) {
        throw new Error(
          "a different publication is already reserved for this inbound event",
        );
      }
      const receipt = JSON.parse(
        await fs.readFile(path.join(directory, "receipt.json"), "utf8"),
      );
      return { directory, existingReceipt: receipt };
    } catch (readError) {
      if (readError.code === "ENOENT") {
        throw new Error(
          "publication is already reserved; refusing an ambiguous retry",
        );
      }
      throw readError;
    }
  }
}

async function storeReceipt(directory, receipt) {
  const temporary = path.join(directory, `receipt.${process.pid}.tmp`);
  await fs.writeFile(temporary, `${JSON.stringify(receipt)}\n`, {
    mode: 0o600,
  });
  await fs.rename(temporary, path.join(directory, "receipt.json"));
}

export function createBuzzTools({
  getContext,
  env = process.env,
  runCommand = run,
} = {}) {
  const buzzReply = defineTool({
    name: "buzz_reply",
    label: "Reply in Buzz",
    description:
      "Publish exactly one non-empty reply to the authenticated inbound Buzz event. Routing is fixed by the harness; do not supply channel or event IDs.",
    parameters: Type.Object({
      content: Type.String({ minLength: 1, maxLength: MAX_CONTENT_BYTES }),
    }),
    execute: async (_toolCallId, params, signal) => {
      const content = params.content;
      if (typeof content !== "string" || content.trim().length === 0) {
        throw new Error("buzz_reply content must not be blank");
      }
      if (Buffer.byteLength(content, "utf8") > MAX_CONTENT_BYTES) {
        throw new Error(
          `buzz_reply content exceeds ${MAX_CONTENT_BYTES} bytes`,
        );
      }
      const context = validateContext(getContext?.());
      const reservation = await reservePublication(context, content, env);
      if (reservation.existingReceipt) {
        return toolText(JSON.stringify(reservation.existingReceipt), {
          receipt: reservation.existingReceipt,
          replay: true,
        });
      }
      const command = env.PI_ACP_BUZZ_COMMAND || "buzz";
      const result = await runCommand(
        command,
        [
          "--format",
          "compact",
          "messages",
          "send",
          "--channel",
          context.channelId,
          "--reply-to",
          context.replyTo,
          "--content",
          "-",
        ],
        { input: content, signal, env },
      );
      let response;
      try {
        response = JSON.parse(result.stdout);
      } catch {
        throw new Error(
          "buzz_reply returned a non-JSON receipt; publication remains reserved",
        );
      }
      if (
        typeof response.event_id !== "string" ||
        !HEX_EVENT.test(response.event_id)
      ) {
        throw new Error(
          "buzz_reply receipt is missing a valid event_id; publication remains reserved",
        );
      }
      if (response.accepted === false) {
        throw new Error(
          "Buzz rejected the publication; reservation retained for operator review",
        );
      }
      const receipt = {
        event_id: response.event_id,
        accepted: true,
        channel_id: context.channelId,
        reply_to: context.replyTo,
        triggering_event_ids: context.triggeringEventIds,
      };
      await storeReceipt(reservation.directory, receipt);
      return toolText(JSON.stringify(receipt), { receipt, replay: false });
    },
  });

  const kanbanTasks = defineTool({
    name: "kanban_tasks",
    label: "Read compact Kanban tasks",
    description:
      "Read one bounded, filtered compact Kanban AI task list. Never downloads the full board.",
    parameters: Type.Object({
      sprint: Type.Optional(Type.Integer({ minimum: 1 })),
      status: Type.Optional(
        Type.Union([
          Type.Literal("todo"),
          Type.Literal("in-progress"),
          Type.Literal("done"),
        ]),
      ),
      channel: Type.Optional(Type.String({ minLength: 1, maxLength: 128 })),
      search: Type.Optional(Type.String({ minLength: 1, maxLength: 256 })),
      limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 20 })),
    }),
    execute: async (_toolCallId, params, signal) => {
      const command =
        env.PI_ACP_KANBAN_COMMAND ||
        path.join(os.homedir(), ".buzz", "bin", "kanban-ai");
      const args = ["tasks", "--limit", String(params.limit || 10), "--json"];
      if (params.sprint) args.push("--sprint", String(params.sprint));
      if (params.status) args.push("--status", params.status);
      if (params.channel) args.push("--channel", params.channel);
      if (params.search) args.push("--search", params.search);
      const result = await runCommand(command, args, { signal, env });
      JSON.parse(result.stdout);
      return toolText(result.stdout, {
        bounded: true,
        staleProvenanceIncluded: true,
      });
    },
  });

  return [buzzReply, kanbanTasks];
}

export const testOnly = { validateContext, reservationKey };
