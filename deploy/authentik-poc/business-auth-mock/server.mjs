import { randomBytes, randomUUID } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import { createServer } from 'node:http';
import { DatabaseSync } from 'node:sqlite';
import * as oidc from 'openid-client';
import {
  EMBED_CODE_PATTERN,
  classifyEmbedSession,
  hashEmbedCode,
  safeEmbedTarget,
} from './embed-session-policy.mjs';

const required = (name) => {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required`);
  return value;
};

const issuer = new URL(required('BUSINESS_OIDC_ISSUER'));
const clientId = required('BUSINESS_OIDC_CLIENT_ID');
const clientSecret = required('BUSINESS_OIDC_CLIENT_SECRET');
const redirectUri = required('BUSINESS_OIDC_REDIRECT_URI');
const postLogoutRedirectUri = required('BUSINESS_POST_LOGOUT_REDIRECT_URI');
const embedAudience = process.env.BUSINESS_EMBED_AUDIENCE?.trim() || 'business-dock';
const embedCallback = new URL(
  process.env.BUSINESS_EMBED_CALLBACK_URI?.trim() || 'buzz-dev://auth/business-bootstrap',
);
if (
  embedCallback.protocol !== 'buzz-dev:' ||
  embedCallback.host !== 'auth' ||
  embedCallback.pathname !== '/business-bootstrap' ||
  embedCallback.search ||
  embedCallback.hash
) {
  throw new Error('BUSINESS_EMBED_CALLBACK_URI must be buzz-dev://auth/business-bootstrap');
}

const workbenchOriginUrl = new URL(required('WORKBENCH_ORIGIN'));
if (
  !['http:', 'https:', 'tauri:'].includes(workbenchOriginUrl.protocol) ||
  !['', '/'].includes(workbenchOriginUrl.pathname) ||
  workbenchOriginUrl.username ||
  workbenchOriginUrl.password ||
  workbenchOriginUrl.search ||
  workbenchOriginUrl.hash
) {
  throw new Error('WORKBENCH_ORIGIN must be an HTTP(S) or Tauri origin');
}
const workbenchOrigin = workbenchOriginUrl.origin;
const businessOrigin = new URL(redirectUri).origin;
const secureCookie = process.env.BUSINESS_COOKIE_SECURE !== 'false';
const sameSite = process.env.BUSINESS_COOKIE_SAME_SITE ?? 'Lax';
if (!['Lax', 'Strict', 'None'].includes(sameSite)) {
  throw new Error('BUSINESS_COOKIE_SAME_SITE must be Lax, Strict, or None');
}
const port = Number(process.env.PORT || 3000);
const sessions = new Map();
const transactions = new Map();
const sessionTtlMs = 15 * 60 * 1000;
const embedTtlMs = 30 * 1000;
let discovered;

const database = new DatabaseSync(
  process.env.BUSINESS_SESSION_DB || '/data/business-sessions.sqlite',
);
database.exec('PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;');
const migrationDirectory = new URL('./migrations/', import.meta.url);
database.exec(
  `CREATE TABLE IF NOT EXISTS schema_migrations (
     name TEXT PRIMARY KEY,
     applied_at INTEGER NOT NULL
   )`,
);
for (const migration of readdirSync(migrationDirectory)
  .filter((name) => name.endsWith('.sql'))
  .sort()) {
  const applied = database.prepare(
    'SELECT 1 FROM schema_migrations WHERE name = ?',
  ).get(migration);
  if (applied) continue;
  database.exec('BEGIN IMMEDIATE');
  try {
    database.exec(readFileSync(new URL(migration, migrationDirectory), 'utf8'));
    database.prepare(
      'INSERT INTO schema_migrations (name, applied_at) VALUES (?, ?)',
    ).run(migration, Date.now());
    database.exec('COMMIT');
  } catch (error) {
    database.exec('ROLLBACK');
    throw error;
  }
}

const configuration = () =>
  (discovered ??= oidc.discovery(issuer, clientId, clientSecret));

function cookies(request) {
  return Object.fromEntries(
    (request.headers.cookie ?? '')
      .split(';')
      .map((entry) => entry.trim().split('='))
      .filter(([name, value]) => name && value)
      .map(([name, value]) => [name, decodeURIComponent(value)]),
  );
}

function cookie(name, value, { maxAge } = {}) {
  return `${name}=${encodeURIComponent(value)}; Path=/; HttpOnly; SameSite=${sameSite}${secureCookie ? '; Secure' : ''}${maxAge !== undefined ? `; Max-Age=${maxAge}` : ''}`;
}

function noStoreRedirect(response, location, headers = {}) {
  response.writeHead(302, {
    'cache-control': 'no-store',
    location,
    'referrer-policy': 'no-referrer',
    ...headers,
  });
  response.end();
}

function html(response, status, body, headers = {}) {
  response.writeHead(status, {
    'cache-control': 'no-store',
    'content-security-policy': `default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-ancestors 'self' ${workbenchOrigin}`,
    'content-type': 'text/html; charset=utf-8',
    'referrer-policy': 'no-referrer',
    'x-content-type-options': 'nosniff',
    ...headers,
  });
  response.end(body);
}

function json(response, status, body) {
  response.writeHead(status, {
    'cache-control': 'no-store',
    'content-type': 'application/json; charset=utf-8',
    'x-content-type-options': 'nosniff',
  });
  response.end(JSON.stringify(body));
}

function liveSession(id) {
  const session = sessions.get(id);
  if (!session || Date.now() - session.createdAt > sessionTtlMs) {
    if (id) sessions.delete(id);
    return null;
  }
  return session;
}

function safeTargetPath(value) {
  return safeEmbedTarget(value, businessOrigin);
}

function createEmbedSession(sessionId, targetPath) {
  const code = randomBytes(32).toString('base64url');
  const id = randomUUID();
  const now = Date.now();
  database.prepare(
    `INSERT INTO embed_sessions
      (id, workbench_session_id, code_hash, target_path, audience,
       expires_at, used_at, status, created_at)
     VALUES (?, ?, ?, ?, ?, ?, NULL, 'pending', ?)`,
  ).run(
    id, sessionId, hashEmbedCode(code), targetPath, embedAudience,
    now + embedTtlMs, now,
  );
  return { code, id };
}

function embedCallbackLocation(code) {
  const callback = new URL(embedCallback);
  callback.searchParams.set('code', code);
  return callback.href;
}

function createBrowserSession(claims) {
  const sessionId = randomUUID();
  const groups = Array.isArray(claims.groups)
    ? claims.groups.filter((value) => typeof value === 'string')
    : [];
  sessions.set(sessionId, {
    subject: claims.sub,
    displayName: claims.name || claims.preferred_username || claims.sub,
    groupsClaimVerified: ['bizfin-finance', 'bizfin-business'].every((group) =>
      groups.includes(group)),
    createdAt: Date.now(),
  });
  return sessionId;
}

function redeemEmbedSession(code) {
  if (!EMBED_CODE_PATTERN.test(code)) return { error: 'invalid' };
  const now = Date.now();
  database.exec('BEGIN IMMEDIATE');
  try {
    const row = database.prepare(
      `SELECT id, workbench_session_id, target_path, audience,
              expires_at, status
       FROM embed_sessions WHERE code_hash = ?`,
    ).get(hashEmbedCode(code));
    const classification = classifyEmbedSession(row, now, embedAudience);
    if (classification === 'invalid') {
      database.exec('ROLLBACK');
      return { error: 'invalid' };
    }
    if (classification === 'used') {
      database.exec('ROLLBACK');
      return { error: 'used' };
    }
    if (classification === 'revoked') {
      database.exec('ROLLBACK');
      return { error: 'revoked' };
    }
    if (classification === 'expired') {
      database.prepare("UPDATE embed_sessions SET status = 'expired' WHERE id = ?").run(row.id);
      database.exec('COMMIT');
      return { error: 'expired' };
    }
    const identity = liveSession(row.workbench_session_id);
    if (!identity) {
      database.prepare("UPDATE embed_sessions SET status = 'expired' WHERE id = ?").run(row.id);
      database.exec('COMMIT');
      return { error: 'expired' };
    }
    const update = database.prepare(
      `UPDATE embed_sessions SET status = 'used', used_at = ?
       WHERE id = ? AND status = 'pending' AND expires_at > ?`,
    ).run(now, row.id, now);
    if (update.changes !== 1) {
      database.exec('ROLLBACK');
      return { error: 'used' };
    }
    database.exec('COMMIT');
    return { identity, targetPath: row.target_path };
  } catch (error) {
    database.exec('ROLLBACK');
    throw error;
  }
}

function sweepExpiredState() {
  const now = Date.now();
  for (const [id, transaction] of transactions) {
    if (now - transaction.createdAt > 10 * 60 * 1000) transactions.delete(id);
  }
  for (const [id, session] of sessions) {
    if (now - session.createdAt > sessionTtlMs) sessions.delete(id);
  }
  database.prepare(
    "UPDATE embed_sessions SET status = 'expired' WHERE status = 'pending' AND expires_at <= ?",
  ).run(now);
  database.prepare('DELETE FROM embed_sessions WHERE created_at < ?').run(
    now - 24 * 60 * 60 * 1000,
  );
}

function page(session) {
  return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Business Auth Mock</title><style>body{font:15px system-ui;margin:0;padding:32px;background:#f6f5ef;color:#24231f}.card{max-width:720px;margin:auto;padding:28px;border:1px solid #bbb8aa;border-radius:16px;background:white}button,a{display:inline-block;margin:6px;padding:10px 14px;border:1px solid #777;border-radius:8px;background:white;color:inherit;text-decoration:none}</style></head><body><main class="card"><h1>Business Auth Mock</h1><p id="status">${session ? `Authenticated as ${session.displayName}` : 'No Business session'}</p><p id="claim-status">${session ? `Groups claim ${session.groupsClaimVerified ? 'verified' : 'missing'}` : ''}</p>${session ? '<a href="/auth/logout">Log out Business</a>' : '<a href="/auth/login">Sign in with Authentik</a>'}<button id="expire">Expire session</button></main><script>const allowedHostOrigin=${JSON.stringify(workbenchOrigin)};let nonce='';const send=(type,payload={})=>parent.postMessage({version:3,type,requestId:crypto.randomUUID(),sessionNonce:nonce,payload},allowedHostOrigin);const csrf=()=>document.cookie.split(';').map(value=>value.trim().split('=')).find(([name])=>name==='__Host-bizfin_csrf')?.[1];const logout=()=>{const token=csrf();return fetch('/api/logout',{method:'POST',headers:token?{'X-CSRF-Token':decodeURIComponent(token)}:{}})};const check=async()=>{const response=await fetch('/api/session',{cache:'no-store'});const identity=await response.json();identity.authenticated?send('AUTH_STATUS',{authenticated:true,user:{subject:identity.subject,displayName:identity.displayName}}):send('AUTH_REQUIRED',{reason:'Business has no authenticated HttpOnly session.'})};addEventListener('message',async(event)=>{const message=event.data;if(event.source!==parent||event.origin!==allowedHostOrigin||!message||message.version!==3||typeof message.sessionNonce!=='string')return;nonce=message.sessionNonce;if(message.type==='CHECK_AUTH')await check();if(message.type==='LOGOUT'){await logout();send('AUTH_STATUS',{authenticated:false})}});document.querySelector('#expire').onclick=async()=>{await logout();send('SESSION_EXPIRED',{reason:'Session expired by the POC control.'})};</script></body></html>`;
}

async function startOidc(response, mode, targetPath) {
  const config = await configuration();
  const state = randomBytes(32).toString('base64url');
  const nonce = randomBytes(32).toString('base64url');
  const codeVerifier = oidc.randomPKCECodeVerifier();
  const codeChallenge = await oidc.calculatePKCECodeChallenge(codeVerifier);
  const transactionId = randomUUID();
  transactions.set(transactionId, {
    state, nonce, codeVerifier, mode, targetPath, createdAt: Date.now(),
  });
  const target = oidc.buildAuthorizationUrl(config, {
    redirect_uri: redirectUri,
    scope: 'openid profile',
    response_type: 'code',
    code_challenge: codeChallenge,
    code_challenge_method: 'S256',
    state,
    nonce,
  });
  noStoreRedirect(response, target.href, {
    'set-cookie': cookie('business_auth_tx', transactionId, { maxAge: 600 }),
  });
}

const server = createServer(async (request, response) => {
  try {
    const requestUrl = new URL(request.url, redirectUri);
    sweepExpiredState();
    const jar = cookies(request);
    const session = liveSession(jar.business_session);

    if (requestUrl.pathname === '/auth/login') {
      await startOidc(
        response,
        requestUrl.searchParams.get('popup') === '1' ? 'web-popup' : 'web',
        '/',
      );
      return;
    }
    if (requestUrl.pathname === '/auth/embed-login') {
      const targetPath = safeTargetPath(requestUrl.searchParams.get('target'));
      if (!targetPath) {
        html(response, 400, '<h1>Invalid Business target</h1>');
        return;
      }
      if (session && jar.business_session) {
        const ticket = createEmbedSession(jar.business_session, targetPath);
        noStoreRedirect(response, embedCallbackLocation(ticket.code));
        return;
      }
      await startOidc(response, 'embed', targetPath);
      return;
    }
    if (requestUrl.pathname === '/auth/callback') {
      const transaction = transactions.get(jar.business_auth_tx);
      transactions.delete(jar.business_auth_tx);
      if (!transaction || Date.now() - transaction.createdAt > 600_000) {
        throw new Error('Missing or expired login transaction');
      }
      const config = await configuration();
      const tokens = await oidc.authorizationCodeGrant(config, requestUrl, {
        pkceCodeVerifier: transaction.codeVerifier,
        expectedState: transaction.state,
        expectedNonce: transaction.nonce,
      });
      const claims = tokens.claims();
      if (!claims?.sub) throw new Error('ID token did not contain a subject');
      const sessionId = createBrowserSession(claims);
      const sessionCookies = [
        cookie('business_session', sessionId, { maxAge: 900 }),
        cookie('business_auth_tx', '', { maxAge: 0 }),
      ];
      if (transaction.mode === 'web-popup') {
        html(
          response,
          200,
          '<!doctype html><title>Business SSO complete</title><p>Business SSO complete. This window can be closed.</p><script>window.close()</script>',
          { 'set-cookie': sessionCookies },
        );
        return;
      }
      const location = transaction.mode === 'embed'
        ? embedCallbackLocation(createEmbedSession(sessionId, transaction.targetPath).code)
        : '/';
      noStoreRedirect(response, location, {
        'set-cookie': sessionCookies,
      });
      return;
    }
    if (requestUrl.pathname === '/api/embed-sessions' && request.method === 'POST') {
      if (!session || !jar.business_session) {
        json(response, 401, { error: 'Business authentication required' });
        return;
      }
      if (request.headers.origin && request.headers.origin !== businessOrigin) {
        json(response, 403, { error: 'Origin rejected' });
        return;
      }
      let rawBody = '';
      for await (const chunk of request) {
        rawBody += chunk;
        if (rawBody.length > 4096) throw new Error('Request body too large');
      }
      const body = rawBody ? JSON.parse(rawBody) : {};
      const targetPath = safeTargetPath(body.target);
      if (!targetPath) {
        json(response, 400, { error: 'Invalid Business target' });
        return;
      }
      const ticket = createEmbedSession(jar.business_session, targetPath);
      json(response, 201, {
        id: ticket.id,
        embedUrl: `${businessOrigin}/embed/bootstrap?code=${encodeURIComponent(ticket.code)}`,
        expiresIn: embedTtlMs / 1000,
      });
      return;
    }
    const revokeMatch = requestUrl.pathname.match(/^\/api\/embed-sessions\/([0-9a-f-]{36})\/revoke$/);
    if (revokeMatch && request.method === 'POST') {
      if (!session || !jar.business_session) {
        json(response, 401, { error: 'Business authentication required' });
        return;
      }
      if (request.headers.origin && request.headers.origin !== businessOrigin) {
        json(response, 403, { error: 'Origin rejected' });
        return;
      }
      const result = database.prepare(
        `UPDATE embed_sessions SET status = 'revoked'
         WHERE id = ? AND workbench_session_id = ? AND status = 'pending'`,
      ).run(revokeMatch[1], jar.business_session);
      if (result.changes !== 1) {
        json(response, 404, { error: 'Pending Embed Session not found' });
        return;
      }
      response.writeHead(204, { 'cache-control': 'no-store' });
      response.end();
      return;
    }
    if (requestUrl.pathname === '/embed/bootstrap') {
      const result = redeemEmbedSession(requestUrl.searchParams.get('code') || '');
      if ('error' in result) {
        html(
          response,
          result.error === 'invalid' ? 404 : 410,
          '<h1>Embed session unavailable</h1><p>Return to Business Dock and retry once.</p>',
        );
        return;
      }
      const sessionId = randomUUID();
      sessions.set(sessionId, { ...result.identity, createdAt: Date.now() });
      noStoreRedirect(response, result.targetPath, {
        'set-cookie': cookie('business_session', sessionId, { maxAge: 900 }),
      });
      return;
    }
    if (requestUrl.pathname === '/api/session') {
      json(response, 200, session ? {
        authenticated: true,
        subject: session.subject,
        displayName: session.displayName,
      } : { authenticated: false });
      return;
    }
    if (requestUrl.pathname === '/api/logout' && request.method === 'POST') {
      if (jar.business_session) sessions.delete(jar.business_session);
      response.writeHead(204, {
        'cache-control': 'no-store',
        'set-cookie': cookie('business_session', '', { maxAge: 0 }),
      });
      response.end();
      return;
    }
    if (requestUrl.pathname === '/auth/logout') {
      if (jar.business_session) sessions.delete(jar.business_session);
      const config = await configuration();
      const target = oidc.buildEndSessionUrl(config, {
        post_logout_redirect_uri: postLogoutRedirectUri,
      });
      noStoreRedirect(response, target.href, {
        'set-cookie': cookie('business_session', '', { maxAge: 0 }),
      });
      return;
    }
    html(response, 200, page(session));
  } catch (error) {
    console.error(
      'Business Auth Mock request failed:',
      error instanceof Error ? error.name : 'UnknownError',
    );
    html(
      response,
      500,
      '<h1>Business Auth Mock failed</h1><p>Check server logs for the non-sensitive error.</p>',
    );
  }
});

server.listen(port, '0.0.0.0', () =>
  console.log(`Business Auth Mock listening on ${port}`),
);
