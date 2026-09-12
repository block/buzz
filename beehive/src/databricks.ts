import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, lstatSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readSettings, saveSettings, settingsId, settingsLock, type ProviderReference } from './settings.ts';
import { retainCredentialAttempt } from './settings-credentials.ts';

/** No discovery or credentials on passive render. Workspace identity is an origin. */
export function databricksHost(input: string): string {
  if (/[\x00-\x20\x7f]/.test(input)) throw Error('Enter a workspace URL without whitespace or controls');
  let u: URL; try { u = new URL(input); } catch { throw Error('Enter a Databricks HTTPS workspace URL'); }
  if (u.protocol !== 'https:' || u.username || u.password || u.search || u.hash || u.port || u.pathname !== '/' || !u.hostname.includes('.') || /^[\d.]+$/.test(u.hostname) || u.hostname.includes(':')) throw Error('Enter an HTTPS workspace origin without credentials, port, path or query');
  return u.origin;
}
/** One native shared-Rust PKCE/catalog invocation. Only private pipes carry the
 * launch bearer; stderr is discarded, capture is capped, cancellation awaits close. */
export async function databricksNative(input: { action: 'login' | 'models' | 'token'; host: string; key: ProviderReference }, signal: AbortSignal, executable = fileURLToPath(new URL('../bin/beehive-databricks', import.meta.url))): Promise<{ ok: true; secret?: string; models?: string[] }> {
  signal.throwIfAborted();
  const host = databricksHost(input.host);
  if (input.key.service !== 'beehive' || !/^provider:[0-9a-f-]{36}$/.test(input.key.account)) throw Error('Invalid Databricks credential reference');
  const root = join(tmpdir(),`beehive-oauth-${process.getuid?.()}`);
  mkdirSync(root,{ recursive: true, mode: 0o700 });
  const st = lstatSync(root); if (!st.isDirectory() || st.uid !== process.getuid?.() || (st.mode & 0o077)) throw Error('Unsafe OAuth coordination directory');
  const coordination = mkdtempSync(join(root,'attempt-'));
  try {
    return await new Promise((resolve,reject) => {
      const child = spawn(executable,[],{ stdio: ['pipe','pipe','ignore'], env: { PATH:'/usr/bin:/bin', ...(process.env.HOME ? { HOME:process.env.HOME } : {}) } });
      let output = '', failed = false;
      const cancel = () => { failed = true; child.kill('SIGTERM'); escalation ??= setTimeout(() => child.kill('SIGKILL'),250); };
      let escalation: ReturnType<typeof setTimeout> | undefined;
      const timer = setTimeout(cancel,input.action === 'login' ? 180000 : 20000);
      signal.addEventListener('abort',cancel,{ once:true });
      child.on('error',() => { failed = true; }); child.stdin.on('error',cancel);
      child.stdout.on('data',(chunk: Buffer) => { if (failed) return; if (Buffer.byteLength(output) + chunk.length > 1048576) { output = ''; cancel(); } else output += chunk.toString(); });
      child.once('close',code => {
        clearTimeout(timer); clearTimeout(escalation); signal.removeEventListener('abort',cancel);
        try { if (failed || signal.aborted || code !== 0) throw Error(); const value = JSON.parse(output); output = ''; if (value.ok !== true) throw Error(); resolve(value); }
        catch { output = ''; reject(Error('Databricks sign-in or credential access failed/cancelled. No provider was saved by this operation. OS credentials may have changed; retry explicit sign-in.')); }
      });
      if (signal.aborted) cancel(); else child.stdin.end(JSON.stringify({ action:input.action, host, account:input.key.account, coordination }));
    });
  } finally { rmSync(coordination,{ recursive:true,force:true }); }
}
/** Native grant and OS read-back precede public CAS; cancellation cannot publish. */
export async function addDatabricks(directory: string, name: string, endpoint: string, signal: AbortSignal, native = databricksNative) {
  if (!name || name.length > 128 || /[\x00-\x1f\x7f]/.test(name)) throw Error('Enter a provider name');
  endpoint = databricksHost(endpoint);
  const previous = readSettings(directory), id = settingsId();
  const key: ProviderReference = { service:'beehive',account:`provider:${id}` };
  settingsLock(directory,() => retainCredentialAttempt(directory,key));
  await native({ action:'login',host:endpoint,key },signal); signal.throwIfAborted();
  return saveSettings(directory,{ ...previous,providers:[...previous.providers,{ id,name,type:'databricks_v2',endpoint,key }] },previous.revision);
}
