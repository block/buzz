import { accessSync, constants, realpathSync, statSync } from 'node:fs';
import { delimiter, join } from 'node:path';

/** Resolve supported Buzz Agent executable without running an installed harness.
 * Other Desktop adapters require separate native launch contracts. */
export function detectBuzzAgent(path = process.env.PATH ?? ''): string | undefined {
  for (const folder of path.split(delimiter).filter(p => p.startsWith('/')).slice(0,128)) {
    try { const executable = realpathSync(join(folder,'buzz-agent')); accessSync(executable,constants.X_OK); if (statSync(executable).isFile()) return executable; } catch { /* Missing PATH candidates are not authentication failures. */ }
  }
  return undefined;
}
/** Authenticated OpenAI model list. Empty and error stay distinct; custom is independent. */
export async function openAIModels(secret: string, signal: AbortSignal, fetcher: typeof fetch = fetch): Promise<string[]> {
  const response = await fetcher('https://api.openai.com/v1/models',{ headers: { Authorization: `Bearer ${secret}` }, signal, redirect: 'error' });
  if (!response.ok || !response.body) throw Error('OpenAI model list unavailable');
  const reader = response.body.getReader(); let bytes = 0; const chunks: Uint8Array[] = [];
  try { while (true) { const next = await reader.read(); if (next.done) break; bytes += next.value.length; if (bytes > 1048576) throw Error('OpenAI model list exceeds limit'); chunks.push(next.value); } }
  finally { await reader.cancel(); reader.releaseLock(); }
  try {
    const json = JSON.parse(Buffer.concat(chunks).toString());
    if (!Array.isArray(json.data) || json.data.length > 10000) throw Error();
    const ids: string[] = json.data.map((v: any) => v.id).filter((id: unknown): id is string => typeof id === 'string' && /^[a-zA-Z0-9_.:/-]{1,200}$/.test(id) && !/(embedding|whisper|tts|dall-e|image|audio|realtime|moderation)/i.test(id));
    const all = new Set(ids);
    return [...all].filter(id => { const base = id.replace(/-\d{4}-\d{2}-\d{2}$/,''); return base === id || !all.has(base); }).sort().slice(0,1000);
  } catch { throw Error('OpenAI model list is invalid'); }
}
