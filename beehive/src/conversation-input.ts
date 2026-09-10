import { realpathSync } from 'node:fs';
import { text } from './protocol.ts';
import type { ConversationSetup } from './conversation.ts';

/** Local interactive input only; never provider login, discovery of credentials or launch. */
export async function conversationInput(ui: { question(prompt: string): Promise<string> }): Promise<ConversationSetup> {
  console.log('Normal conversation requires locally installed buzz-acp and Buzz CLI, a trusted conversation relay and local provider setup as the service user. Missing tools: install them locally, or cancel and explicitly choose diagnostic. No login or admission is verified.');
  const executable = realpathSync(text(await ui.question('Absolute installed buzz-acp executable: ')));
  const relay = text(await ui.question('Buzz CONVERSATION relay URL (not the Beehive management relay): '));
  const tool = realpathSync(text(await ui.question('Absolute installed buzz CLI executable: ')));
  return { executable, relay, replyTool: { executable: tool } };
}
