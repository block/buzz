import { accessSync, constants, readFileSync, realpathSync } from 'node:fs';
import { isAbsolute } from 'node:path';
import { createHash } from 'node:crypto';
import { prepareAgent, type AgentLaunch } from './acp.ts';
import { publicKey } from './protocol.ts';

/** Host-local external transport configuration. Never accepted through management messages. */
export type ConversationSetup = { executable: string; relay: string; authTag?: string };
/** Launch intent, not a claim of relay admission or a model-confirmed conversation. */
export type ConversationPlan = ReturnType<typeof prepareConversation>;

/**
 * Exact upstream external contract at 051c3a2: spawn_agent_child, CliArgs,
 * resolve_agent_owner and HarnessRelay::connect. No ambient environment is merged.
 * A NIP-OA tag is an opaque host-local credential; only upstream verifies it.
 */
export function prepareConversation(input: ConversationSetup, agent: AgentLaunch, agentSecret: string, owner: string) {
  const prepared = prepareAgent(agent);
  const agentPublicKey = publicKey(agentSecret);
  if (!/^[a-f0-9]{64}$/.test(owner)) throw Error('Invalid conversation owner public key');
  if (!isAbsolute(input.executable) || realpathSync(input.executable) !== input.executable) throw Error('Conversation executable must be a canonical absolute path');
  accessSync(input.executable, constants.X_OK);
  const url = new URL(input.relay);
  if (!['ws:', 'wss:'].includes(url.protocol) || url.username || url.password || url.search || url.hash) throw Error('Invalid conversation relay URL');
  if (url.protocol === 'ws:' && !['127.0.0.1', '[::1]', 'localhost'].includes(url.hostname)) throw Error('Non-loopback conversation transport requires TLS');
  // Upstream uses comma-delimited arguments, not shell parsing. Reject lossy encoding.
  if (prepared.plan.args.some(a => a.includes(',') || a.includes('\0') || !a.length)) throw Error('ACP arguments cannot contain commas, NUL or empty values');
  if (input.authTag !== undefined && (typeof input.authTag !== 'string' || input.authTag.length > 16384 || !input.authTag.length)) throw Error('Invalid local owner attestation');
  return Object.freeze({
    executable: input.executable,
    executableHash: createHash('sha256').update(readFileSync(input.executable)).digest('hex'),
    workspace: prepared.plan.workspace,
    agentPublicKey,
    owner,
    relay: url.href,
    agentExecutableHash: prepared.executableHash,
    // Authoritative identity/transport applied after provider env; no arbitrary env input.
    env: Object.freeze({ ...prepared.env,
      BUZZ_PRIVATE_KEY: agentSecret,
      BUZZ_RELAY_URL: url.href,
      BUZZ_ACP_AGENT_OWNER: owner,
      ...(input.authTag === undefined ? {} : { BUZZ_AUTH_TAG: input.authTag }),
      BUZZ_ACP_AGENT_COMMAND: prepared.plan.executable,
      BUZZ_ACP_AGENT_ARGS: prepared.plan.args.join(','),
      BUZZ_ACP_MODEL: prepared.plan.model,
      BUZZ_ACP_MCP_COMMAND: '',
      BUZZ_ACP_AGENTS: '1',
      BUZZ_ACP_RESPOND_TO: 'owner-only',
      BUZZ_ACP_ALLOWED_RESPOND_TO: 'owner-only',
      BUZZ_ACP_PERMISSION_MODE: 'default',
    }),
  });
}

/** Safe remote summary. Credentials and executable paths are never included. */
export function conversationSummary(_setup: ConversationSetup) {
  return { transport: 'external-buzz-acp', admission: 'unverified', modelEvidence: 'not-observed', launch: 'host-owned ACP broker; awaiting conversation evidence' };
}
