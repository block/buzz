import { registerHooks } from 'node:module';
import { fileURLToPath } from 'node:url';
const fixture = fileURLToPath(new URL('./manager-installed-fixture.ts', import.meta.url));
// Explicit test-only seam. No production environment switch or OS-store fallback.
registerHooks({ load(url, context, next) {
  const result = next(url, context);
  if (url.endsWith('/src/harness-discovery.ts')) return { ...result, source: "export async function discoverHarnesses() { return [{id:'buzz-agent',label:'Buzz Agent',executable:process.execPath,state:'available',providers:['openai','databricks_v2'],reason:'Synthetic only'}]; }" };
  if (url.endsWith('/src/databricks-runtime.ts') || url.endsWith('/src/databricks-runtime-child.ts')) return { ...result, source: "throw Error('Runtime auth disabled by installed fixture');" };
  if (url.endsWith('/src/databricks.ts')) return { ...result, source: "export function databricksHost(v) { return new URL(v).origin; } export async function databricksNative() { throw Error('Native OAuth disabled by installed fixture'); } export async function addDatabricks() { throw Error('Native OAuth disabled by installed fixture'); }" };
  if (url.endsWith('/src/agent-profile.ts')) return { ...result, source: "export async function fetchAgentProfile() { return {profileState:'none'}; }" };
  if (url.endsWith('/src/host-service.ts')) return { ...result, source: "export async function serviceStatus() { return {state:'stopped',relay:'disconnected'}; } export async function startService() { throw Error('Service start disabled by installed fixture'); } export async function stopService() { throw Error('Service stop disabled by installed fixture'); }" };
  if (url.endsWith('/src/settings-models.ts')) return { ...result, source: "export function detectBuzzAgent() { return process.execPath; } export async function openAIModels() { throw Error('Direct provider access disabled by installed fixture'); }" };
  if (!url.endsWith('/src/manager-entry.ts')) return result;
  const source = String(result.source);
  const original = 'new ManagerController(homedir(), snapshot => send({ snapshot }))';
  if (!source.includes(original)) throw Error('Installed manager fixture failed closed');
  return { ...result, source: `import { fixtureCredential, disconnectedTransport } from ${JSON.stringify(fixture)};\n` + source.replace(original, 'new ManagerController(homedir(), snapshot => send({ snapshot }), fixtureCredential, disconnectedTransport)') };
} });
