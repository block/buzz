import { appendFileSync } from 'node:fs';
if (process.env.BEEHIVE_FIXTURE !== '1') throw Error('Fixture only');
if (!process.argv.includes('--descendant')) appendFileSync('binding-B.jsonl', JSON.stringify({ binding: 'B', pid: process.pid, instructions: process.env.BUZZ_AGENT_SYSTEM_PROMPT ?? null }) + '\n');
await import('./runner.ts');
