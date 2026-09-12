import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { randomBytes, timingSafeEqual } from 'node:crypto';
import { databricksHost, databricksNative } from './databricks.ts';
import type { ProviderReference } from './settings.ts';

const posts = new Set(['/ai-gateway/openai/v1/responses', '/ai-gateway/anthropic/v1/messages', '/ai-gateway/mlflow/v1/chat/completions']);
const catalogs = new Set(['/api/ai-gateway/v2/endpoints', '/api/2.1/unity-catalog/model-services']);
/** Run-local transport adapter. Native Rust still owns OAuth/refresh and OS custody;
 * the stock harness retains its canonical Databricks v2 routing and wire bodies.
 * Only the private loopback capability reaches the harness, never a provider token.
 */
export async function databricksRuntime(input: { host: string; key: ProviderReference; model: string }, native = databricksNative, fetcher: typeof fetch = fetch) {
  const host = databricksHost(input.host), capability = randomBytes(32).toString('hex');
  const active = new Set<AbortController>(), jobs = new Set<Promise<void>>();
  let stopped = false, denied = false, closing: Promise<void> | undefined;
  const server = createServer((req, res) => {
    const job = serve(req, res); jobs.add(job); void job.finally(() => jobs.delete(job));
  });
  server.headersTimeout = 5000; server.requestTimeout = 20000; server.keepAliveTimeout = 1000;
  server.maxConnections = 16;
  const error = (res: ServerResponse, code: number) => { if (!res.destroyed) { res.writeHead(code, { 'content-type': 'application/json' }); res.end(JSON.stringify({ error: code === 401 ? 'Databricks authentication rejected; explicit sign-in may be required' : 'Databricks runtime request unavailable; credential refresh or network access failed; explicit sign-in may be required' })); } };
  async function serve(req: IncomingMessage, res: ServerResponse) {
    const supplied = Buffer.from(req.headers.authorization ?? ''), expected = Buffer.from(`Bearer ${capability}`);
    if (stopped || supplied.length !== expected.length || !timingSafeEqual(supplied, expected)) { error(res, 403); return; }
    if (denied) { error(res, 401); return; }
    if (active.size >= 8) { error(res, 503); return; }
    const controller = new AbortController(); active.add(controller);
    const abort = () => controller.abort();
    req.once('aborted', abort); res.once('close', abort);
    const timer = setTimeout(abort, 120000);
    try {
      const url = new URL(req.url ?? '', 'http://127.0.0.1');
      if (url.href.length > 8192 || url.origin !== 'http://127.0.0.1' || url.hash || (req.method === 'POST' ? !posts.has(url.pathname) || !!url.search : req.method !== 'GET' || !catalogs.has(url.pathname) || [...url.searchParams.keys()].some(k => !['page_token', 'page_size', 'view'].includes(k) || k === 'page_size' && url.searchParams.get(k) !== '100' || k === 'view' && url.searchParams.get(k) !== 'FULL'))) { error(res, 403); return; }
      let body: Buffer | undefined;
      if (req.method === 'POST') {
        const chunks: Buffer[] = []; let size = 0;
        for await (const chunk of req) { size += chunk.length; if (size > 16 * 1024 * 1024) throw Error(); chunks.push(chunk); }
        body = Buffer.concat(chunks);
        if (JSON.parse(body.toString()).model !== input.model) { error(res, 403); return; }
      }
      controller.signal.throwIfAborted();
      // Every request re-enters the workspace-bound native token source. Its
      // cross-process lock serializes rotated grants across concurrent runtimes.
      const token = await native({ action: 'token', host, key: input.key }, controller.signal);
      controller.signal.throwIfAborted();
      if (!token.secret || /\s/.test(token.secret)) throw Error();
      const headers = new Headers({ authorization: `Bearer ${token.secret}`, 'content-type': 'application/json' });
      if (url.pathname === '/ai-gateway/anthropic/v1/messages') headers.set('anthropic-version', '2023-06-01');
      token.secret = undefined;
      const upstream = await fetcher(host + url.pathname + url.search, { method: req.method, headers, ...(body ? { body: body.toString() } : {}), signal: controller.signal, redirect: 'error' });
      if (upstream.status === 401 || upstream.status === 403) { denied = true; await upstream.body?.cancel(); error(res, 401); return; }
      if (!upstream.ok || !upstream.body) { await upstream.body?.cancel(); error(res, 502); return; }
      // The canonical buzz-agent consumer is non-streaming. Bound before exposing
      // a response, and never relay upstream diagnostics or redirect headers.
      const reader = upstream.body.getReader(), chunks: Uint8Array[] = []; let size = 0;
      try { while (true) { const next = await reader.read(); if (next.done) break; size += next.value.length; if (size > 16 * 1024 * 1024) throw Error(); chunks.push(next.value); } }
      finally { await reader.cancel(); reader.releaseLock(); }
      controller.signal.throwIfAborted();
      if (stopped || res.destroyed) return;
      res.writeHead(200, { 'content-type': upstream.headers.get('content-type')?.startsWith('text/event-stream') ? 'text/event-stream' : 'application/json' }); res.end(Buffer.concat(chunks));
    } catch { error(res, 503); }
    finally { clearTimeout(timer); req.removeListener('aborted', abort); res.removeListener('close', abort); active.delete(controller); }
  }
  await new Promise<void>((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const address = server.address();
  if (!address || typeof address === 'string') { server.close(); throw Error('Runtime listener unavailable'); }
  return {
    endpoint: `http://127.0.0.1:${address.port}`, capability,
    /** Fence acceptance first, abort network/helpers, then await their close. */
    stop(): Promise<void> {
      return closing ??= (async () => {
        stopped = true; for (const controller of active) controller.abort();
        server.closeAllConnections();
        await Promise.all([new Promise<void>(resolve => server.close(() => resolve())), ...jobs]);
      })();
    },
  };
}
