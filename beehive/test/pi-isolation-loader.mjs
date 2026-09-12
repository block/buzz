// Explicit test-only subprocess seam: never installed or imported by production.
import { registerHooks } from 'node:module';
import { readFileSync, appendFileSync } from 'node:fs';
import { join } from 'node:path';
const loader = import.meta.url;
registerHooks({ load(url,context,next) {
  const result = next(url,context);
  if (url.endsWith('/src/acp.ts')) return { ...result, source: String(result.source).replace("[fileURLToPath(new URL('./pi-runtime-child.ts', import.meta.url)),", `[ '--import', ${JSON.stringify(loader)}, fileURLToPath(new URL('./pi-runtime-child.ts', import.meta.url)),`) };
  if (url.endsWith('/src/databricks.ts')) {
    const source = String(result.source);
    const start = source.indexOf('export async function databricksNative(');
    const end = source.indexOf('/** Native grant',start);
    if(start<0 || end<0) throw Error('Native isolation seam changed');
    return {...result,source:source.slice(0,start)+`export async function databricksNative(input,signal) { return globalThis.piFixtureNative(input,signal); }\n`+source.slice(end)};
  }
  return result;
}});
let expires = 0, generation = 0;
globalThis.piFixtureNative = async (input,signal) => {
  signal.throwIfAborted();
  const fixture = JSON.parse(readFileSync(join(process.cwd(),'pi-fixture.json'),'utf8'));
  if (input.action !== 'token' || input.host !== 'https://pi-workspace.example' || JSON.stringify(input.key) !== JSON.stringify(fixture.key)) throw Error('Fixture native boundary mismatch');
  if (Date.now() >= expires) { generation++; expires=Date.now()+50; }
  appendFileSync(join(process.cwd(),'native-calls'),JSON.stringify({input,generation})+'\n');
  return {ok:true,secret:`synthetic-pi-generation-${generation}`};
};
const originalFetch = globalThis.fetch;
globalThis.fetch = async (url,options) => {
  if (!String(url).startsWith('https://')) return originalFetch(url,options);
  const fixture = JSON.parse(readFileSync(join(process.cwd(),'pi-fixture.json'),'utf8'));
  if (url !== 'https://pi-workspace.example'+fixture.route) throw Error('Nonfixture network forbidden');
  return originalFetch(fixture.endpoint+fixture.route,options);
};
