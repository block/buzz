import crypto from "node:crypto";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { attachJsonlReader, writeJsonl } from "./jsonl.mjs";

const SDK_BRIDGE_PATH = fileURLToPath(
  new URL("./sdk-bridge.mjs", import.meta.url),
);

const INVALID_REQUEST = -32600;
const METHOD_NOT_FOUND = -32601;
const INVALID_PARAMS = -32602;
const INTERNAL_ERROR = -32603;
const MAX_SYSTEM_PROMPT_BYTES = 64 * 1024;
const MAX_TOOL_OUTPUT_BYTES = 16 * 1024;

function rpcResult(id, result) {
  return { jsonrpc: "2.0", id, result };
}

function rpcError(id, code, message) {
  return { jsonrpc: "2.0", id: id ?? null, error: { code, message } };
}

function promptText(blocks) {
  if (!Array.isArray(blocks)) return null;
  const texts = [];
  for (const block of blocks) {
    if (block?.type !== "text" || typeof block.text !== "string") return null;
    texts.push(block.text);
  }
  const joined = texts.join("\n\n");
  return joined.trim().length > 0 ? joined : null;
}

function boundedText(value) {
  const text =
    typeof value === "string" ? value : JSON.stringify(value ?? null);
  return Buffer.byteLength(text, "utf8") <= MAX_TOOL_OUTPUT_BYTES
    ? text
    : `${Buffer.from(text).subarray(0, MAX_TOOL_OUTPUT_BYTES).toString("utf8")}\n…[truncated]`;
}

function emptyUsage() {
  return {
    input: 0,
    output: 0,
    cacheRead: 0,
    cacheWrite: 0,
    totalTokens: 0,
    totalReliable: true,
    cost: 0,
    model: null,
  };
}

function addUsage(target, usage) {
  if (!usage || typeof usage !== "object") return;
  for (const key of ["input", "output", "cacheRead", "cacheWrite"]) {
    const value = usage[key];
    if (Number.isFinite(value) && value >= 0) target[key] += value;
  }
  if (Number.isFinite(usage.totalTokens) && usage.totalTokens >= 0) {
    target.totalTokens += usage.totalTokens;
  } else {
    target.totalReliable = false;
  }
  const cost = usage.cost?.total;
  if (Number.isFinite(cost) && cost >= 0) target.cost += cost;
}

export class PiAcpAdapter {
  constructor({ input, output, errorOutput, env = process.env }) {
    this.input = input;
    this.output = output;
    this.errorOutput = errorOutput;
    this.env = env;
    this.session = null;
    this.pi = null;
    this.piRequestId = 0;
    this.piResponses = new Map();
    this.currentPrompt = null;
    this.cumulative = emptyUsage();
    this.shuttingDown = false;
  }

  start() {
    attachJsonlReader(
      this.input,
      (message) => void this.#handleAcp(message),
      (error) => this.#send(rpcError(null, -32700, error.message)),
    );
    this.input.on("end", () => void this.shutdown());
  }

  async shutdown() {
    if (this.shuttingDown) return;
    this.shuttingDown = true;
    if (!this.pi) return;
    const child = this.pi;
    this.pi = null;
    try {
      if (process.platform !== "win32" && child.pid)
        process.kill(-child.pid, "SIGTERM");
      else child.kill("SIGTERM");
    } catch {
      // The child has already exited.
    }
  }

  #send(value) {
    writeJsonl(this.output, value);
  }

  #log(message) {
    this.errorOutput.write(`[pi-acp-rpc-spike] ${message}\n`);
  }

  async #handleAcp(message) {
    if (message?.jsonrpc !== "2.0" || typeof message.method !== "string") {
      if (message?.id !== undefined)
        this.#send(
          rpcError(message.id, INVALID_REQUEST, "invalid JSON-RPC request"),
        );
      return;
    }
    try {
      switch (message.method) {
        case "initialize":
          this.#initialize(message);
          break;
        case "session/new":
          await this.#sessionNew(message);
          break;
        case "session/prompt":
          await this.#sessionPrompt(message);
          break;
        case "session/cancel":
          await this.#sessionCancel(message.params);
          break;
        case "_session/steering":
          await this.#steer(message);
          break;
        default:
          if (message.id !== undefined)
            this.#send(
              rpcError(
                message.id,
                METHOD_NOT_FOUND,
                `method not found: ${message.method}`,
              ),
            );
      }
    } catch (error) {
      this.#log(error.stack ?? error.message);
      if (message.id !== undefined)
        this.#send(rpcError(message.id, INTERNAL_ERROR, error.message));
    }
  }

  #initialize(message) {
    const requested = message.params?.protocolVersion;
    if (!Number.isInteger(requested) || requested < 1) {
      this.#send(
        rpcError(
          message.id,
          INVALID_PARAMS,
          "initialize: protocolVersion must be a positive integer",
        ),
      );
      return;
    }
    this.#send(
      rpcResult(message.id, {
        protocolVersion: Math.min(requested, 2),
        agentCapabilities: {
          loadSession: false,
          promptCapabilities: {
            image: false,
            audio: false,
            embeddedContext: false,
          },
          mcpCapabilities: { http: false, sse: false },
        },
        agentInfo: { name: "pi-acp", version: "0.1.0" },
        _meta: {
          steering: { supported: true },
          pilot: { liveCanaryValidated: true, fleetApproved: false },
        },
      }),
    );
  }

  async #sessionNew(message) {
    const cwd = message.params?.cwd;
    const systemPrompt = message.params?.systemPrompt;
    const mcpServers = message.params?.mcpServers;
    if (this.session) {
      this.#send(
        rpcError(
          message.id,
          INVALID_PARAMS,
          "session/new: RPC spike supports one task session per process",
        ),
      );
      return;
    }
    if (typeof cwd !== "string" || !path.isAbsolute(cwd)) {
      this.#send(
        rpcError(
          message.id,
          INVALID_PARAMS,
          "session/new: cwd must be an absolute path",
        ),
      );
      return;
    }
    if (systemPrompt !== undefined && typeof systemPrompt !== "string") {
      this.#send(
        rpcError(
          message.id,
          INVALID_PARAMS,
          "session/new: systemPrompt must be a string",
        ),
      );
      return;
    }
    if (
      Buffer.byteLength(systemPrompt ?? "", "utf8") > MAX_SYSTEM_PROMPT_BYTES
    ) {
      this.#send(
        rpcError(
          message.id,
          INVALID_PARAMS,
          `session/new: systemPrompt exceeds ${MAX_SYSTEM_PROMPT_BYTES} bytes`,
        ),
      );
      return;
    }
    if (!Array.isArray(mcpServers)) {
      this.#send(
        rpcError(
          message.id,
          INVALID_PARAMS,
          "session/new: mcpServers must be an array",
        ),
      );
      return;
    }
    if (mcpServers.length > 0) {
      this.#send(
        rpcError(
          message.id,
          INVALID_PARAMS,
          "session/new: MCP servers are unsupported by the RPC spike",
        ),
      );
      return;
    }

    const sessionId = `pi_${crypto.randomUUID()}`;
    await this.#spawnPi(cwd, systemPrompt);
    this.session = { id: sessionId, cwd };
    this.#send(rpcResult(message.id, { sessionId }));
  }

  async #spawnPi(cwd, systemPrompt) {
    const command = this.env.PI_ACP_PI_COMMAND || process.execPath;
    let configuredArgs;
    try {
      configuredArgs = this.env.PI_ACP_PI_ARGS_JSON
        ? JSON.parse(this.env.PI_ACP_PI_ARGS_JSON)
        : [SDK_BRIDGE_PATH];
    } catch (error) {
      throw new Error(`PI_ACP_PI_ARGS_JSON is invalid: ${error.message}`);
    }
    if (
      !Array.isArray(configuredArgs) ||
      configuredArgs.some((arg) => typeof arg !== "string")
    ) {
      throw new Error("PI_ACP_PI_ARGS_JSON must be a JSON array of strings");
    }
    const args = [...configuredArgs];
    if (this.env.PI_ACP_ALLOW_DISCOVERY !== "1")
      args.push("--no-extensions", "--no-skills", "--no-prompt-templates");
    args.push("--tools", this.env.PI_ACP_TOOLS || "read");
    if (systemPrompt?.trim()) args.push("--system-prompt", systemPrompt);

    const child = spawn(command, args, {
      cwd,
      env: this.env,
      stdio: ["pipe", "pipe", "pipe"],
      detached: process.platform !== "win32",
    });
    this.pi = child;
    child.stderr.on("data", (chunk) => this.errorOutput.write(chunk));
    attachJsonlReader(
      child.stdout,
      (event) => this.#handlePi(event),
      (error) =>
        this.#failPi(new Error(`Pi RPC protocol error: ${error.message}`)),
    );
    child.on("error", (error) => this.#failPi(error));
    child.on("exit", (code, signal) => {
      if (!this.shuttingDown && this.pi === child) {
        this.#failPi(
          new Error(
            `Pi RPC exited unexpectedly (code=${code}, signal=${signal})`,
          ),
        );
      }
    });
    try {
      await this.#sendPiCommand("get_state");
    } catch (error) {
      await this.shutdown();
      this.shuttingDown = false;
      throw new Error(`Pi RPC readiness failed: ${error.message}`);
    }
  }

  #assertSession(params, id, stage) {
    if (!this.session || params?.sessionId !== this.session.id) {
      this.#send(rpcError(id, INVALID_PARAMS, `${stage}: unknown session`));
      return false;
    }
    return true;
  }

  async #sessionPrompt(message) {
    if (!this.#assertSession(message.params, message.id, "session/prompt"))
      return;
    if (this.currentPrompt) {
      this.#send(
        rpcError(message.id, INVALID_PARAMS, "session/prompt: session is busy"),
      );
      return;
    }
    const text = promptText(message.params?.prompt);
    if (!text) {
      this.#send(
        rpcError(
          message.id,
          INVALID_PARAMS,
          "session/prompt: prompt must contain non-empty text blocks",
        ),
      );
      return;
    }
    this.currentPrompt = {
      acpId: message.id,
      cancelled: false,
      usage: emptyUsage(),
      finalStopReason: "end_turn",
    };
    const buzzContext = message.params?._meta?.buzz;
    if (
      !buzzContext ||
      typeof buzzContext.channelId !== "string" ||
      !Array.isArray(buzzContext.triggeringEventIds) ||
      typeof buzzContext.replyTo !== "string"
    ) {
      this.currentPrompt = null;
      this.#send(
        rpcError(
          message.id,
          INVALID_PARAMS,
          "session/prompt: authenticated _meta.buzz routing context is required",
        ),
      );
      return;
    }
    try {
      await this.#sendPiCommand("prompt", { message: text, buzzContext });
    } catch (error) {
      const prompt = this.currentPrompt;
      this.currentPrompt = null;
      if (prompt)
        this.#send(
          rpcError(
            prompt.acpId,
            INTERNAL_ERROR,
            `Pi rejected prompt: ${error.message}`,
          ),
        );
    }
  }

  async #sessionCancel(params) {
    if (
      !this.session ||
      params?.sessionId !== this.session.id ||
      !this.currentPrompt
    )
      return;
    this.currentPrompt.cancelled = true;
    try {
      await this.#sendPiCommand("abort");
    } catch (error) {
      this.#failPi(error);
    }
  }

  async #steer(message) {
    if (!this.#assertSession(message.params, message.id, "steering")) return;
    const text = promptText(message.params?.prompt);
    if (!text) {
      this.#send(
        rpcError(
          message.id,
          INVALID_PARAMS,
          "steering: prompt must contain non-empty text blocks",
        ),
      );
      return;
    }
    if (!this.currentPrompt) {
      this.#send(
        rpcError(message.id, INVALID_PARAMS, "steering: no active prompt"),
      );
      return;
    }
    await this.#sendPiCommand("steer", { message: text });
    this.#send(rpcResult(message.id, { outcome: "injected" }));
  }

  #sendPiCommand(type, fields = {}) {
    if (!this.pi?.stdin.writable)
      return Promise.reject(new Error("Pi RPC is not running"));
    const id = `pi-${++this.piRequestId}`;
    const command = { id, type, ...fields };
    return new Promise((resolve, reject) => {
      this.piResponses.set(id, { resolve, reject, command: type });
      writeJsonl(this.pi.stdin, command);
    });
  }

  #handlePi(event) {
    if (event?.type === "response" && event.id !== undefined) {
      const pending = this.piResponses.get(event.id);
      if (!pending) return;
      this.piResponses.delete(event.id);
      if (event.success) pending.resolve(event.data);
      else
        pending.reject(new Error(event.error || `${pending.command} failed`));
      return;
    }
    const prompt = this.currentPrompt;
    if (!prompt) return;
    switch (event?.type) {
      case "message_update":
        this.#messageUpdate(event.assistantMessageEvent);
        break;
      case "tool_execution_start":
        this.#toolStart(event);
        break;
      case "tool_execution_update":
        this.#toolUpdate(event);
        break;
      case "tool_execution_end":
        this.#toolEnd(event);
        break;
      case "turn_end":
        this.#turnEnd(event);
        break;
      case "agent_settled":
        this.#settlePrompt();
        break;
      case "extension_ui_request":
        if (["select", "confirm", "input", "editor"].includes(event.method)) {
          writeJsonl(this.pi.stdin, {
            type: "extension_ui_response",
            id: event.id,
            cancelled: true,
          });
        }
        break;
      default:
        break;
    }
  }

  #update(update) {
    this.#send({
      jsonrpc: "2.0",
      method: "session/update",
      params: { sessionId: this.session.id, update },
    });
  }

  #messageUpdate(delta) {
    if (delta?.type === "text_delta" && typeof delta.delta === "string") {
      this.#update({
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: delta.delta },
      });
    } else if (
      delta?.type === "thinking_delta" &&
      typeof delta.delta === "string"
    ) {
      this.#update({
        sessionUpdate: "agent_thought_chunk",
        content: { type: "text", text: delta.delta },
      });
    }
  }

  #toolStart(event) {
    this.#update({
      sessionUpdate: "tool_call",
      toolCallId: String(event.toolCallId),
      title: String(event.toolName || "tool"),
      kind: event.toolName === "bash" ? "shell" : "other",
      status: "in_progress",
      rawInput: event.args ?? {},
    });
  }

  #toolUpdate(event) {
    this.#update({
      sessionUpdate: "tool_call_update",
      toolCallId: String(event.toolCallId),
      status: "in_progress",
      content: [
        {
          type: "content",
          content: { type: "text", text: boundedText(event.partialResult) },
        },
      ],
    });
  }

  #toolEnd(event) {
    this.#update({
      sessionUpdate: "tool_call_update",
      toolCallId: String(event.toolCallId),
      status: event.isError ? "failed" : "completed",
      content: [
        {
          type: "content",
          content: { type: "text", text: boundedText(event.result) },
        },
      ],
    });
  }

  #turnEnd(event) {
    const usage = event.message?.usage;
    addUsage(this.currentPrompt.usage, usage);
    if (event.message?.provider && event.message?.model) {
      this.currentPrompt.usage.model = `${event.message.provider}/${event.message.model}`;
    }
    for (const result of event.toolResults ?? [])
      addUsage(this.currentPrompt.usage, result?.usage);
    if (event.message?.stopReason === "length")
      this.currentPrompt.finalStopReason = "max_tokens";
    else if (event.message?.stopReason === "aborted")
      this.currentPrompt.finalStopReason = "cancelled";
  }

  #settlePrompt() {
    const prompt = this.currentPrompt;
    if (!prompt) return;
    this.currentPrompt = null;
    const usage = prompt.usage;
    for (const key of [
      "input",
      "output",
      "cacheRead",
      "cacheWrite",
      "totalTokens",
      "cost",
    ]) {
      this.cumulative[key] += usage[key];
    }
    this.cumulative.totalReliable &&= usage.totalReliable;
    this.cumulative.model = usage.model || this.cumulative.model;
    const update = {
      sessionUpdate: "usage_update",
      used: 0,
      contextLimit: 0,
      accumulatedInputTokens:
        this.cumulative.input +
        this.cumulative.cacheRead +
        this.cumulative.cacheWrite,
      accumulatedOutputTokens: this.cumulative.output,
      accumulatedCachedInputTokens: this.cumulative.cacheRead,
      accumulatedCacheWriteTokens: this.cumulative.cacheWrite,
      accumulatedCost: this.cumulative.cost,
      model: this.cumulative.model,
    };
    if (this.cumulative.totalReliable)
      update.accumulatedTotalTokens = this.cumulative.totalTokens;
    this.#send({
      jsonrpc: "2.0",
      method: "_goose/unstable/session/update",
      params: { sessionId: this.session.id, update },
    });
    this.#send(
      rpcResult(prompt.acpId, {
        stopReason: prompt.cancelled ? "cancelled" : prompt.finalStopReason,
      }),
    );
  }

  #failPi(error) {
    this.#log(error.message);
    for (const pending of this.piResponses.values()) pending.reject(error);
    this.piResponses.clear();
    if (this.currentPrompt) {
      this.#send(
        rpcError(this.currentPrompt.acpId, INTERNAL_ERROR, error.message),
      );
      this.currentPrompt = null;
    }
  }
}
