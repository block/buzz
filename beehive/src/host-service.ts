import { connect, createServer, type Socket } from 'node:net';
import { existsSync, mkdirSync, rmdirSync, unlinkSync, chmodSync, lstatSync } from 'node:fs';
import { join } from 'node:path';
import { randomBytes, createHmac, timingSafeEqual } from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { readPrivate, writePrivate } from './storage.ts';

/** User-private per-instance capability, not a PID-based ownership assertion. */
type Instance = { version: 1; instance: string; token: string; socket: string };
export type ServiceStatus = { state: 'running' | 'stopped' | 'unknown'; instance?: string; relay?: 'connected' | 'disconnected' | 'unknown'; revision?: number; agents?: number };
const recordPath = (directory: string) => join(directory,'service.json');
const guard = (directory: string) => join(directory,'service.lock');
const sign = (token: string, value: unknown) => createHmac('sha256',token).update(JSON.stringify(value)).digest('hex');
function authentic(token: string, value: unknown, mac: unknown) { return typeof mac === 'string' && /^[0-9a-f]{64}$/.test(mac) && timingSafeEqual(Buffer.from(sign(token,value),'hex'),Buffer.from(mac,'hex')); }
function privateDirectory(directory: string) {
  const stat = lstatSync(directory);
  if (!stat.isDirectory() || stat.mode & 0o077 || stat.uid !== process.getuid?.()) throw Error('Host service directory must be owner-only');
}
function instance(directory: string): Instance {
  privateDirectory(directory);
  const value = readPrivate(recordPath(directory)) as Instance;
  if (value.version !== 1 || !/^[0-9a-f]{32}$/.test(value.instance) || !/^[0-9a-f]{64}$/.test(value.token) || value.socket !== join(directory,'service.sock')) throw Error('Unverified service instance');
  return value;
}
/** Authenticate both directions with a fresh nonce. Never send the capability itself. */
async function call(directory: string, action: 'status' | 'stop', expected?: string): Promise<ServiceStatus> {
  const retained = instance(directory);
  if (expected && expected !== retained.instance) throw Error('Host instance changed');
  const request = { action, instance: retained.instance, nonce: randomBytes(16).toString('hex') };
  return new Promise((resolve,reject) => {
    const socket = connect(retained.socket); let buffer = '', result: ServiceStatus | undefined, error: Error | undefined;
    const finish = (e?: Error) => { error ??= e; socket.destroy(); };
    const timer = setTimeout(() => finish(Error('Host response unavailable; status unknown')), action === 'stop' ? 15000 : 1500);
    socket.on('connect', () => socket.write(JSON.stringify({ request, mac: sign(retained.token,request) })+'\n'));
    socket.on('error', () => { error ??= Error('Host endpoint unavailable; status unknown'); });
    socket.on('data', bytes => {
      if (buffer.length + bytes.length > 8192) return finish(Error('Invalid host response'));
      buffer += bytes.toString(); if (!buffer.includes('\n')) return;
      try { const value = JSON.parse(buffer.trim()); if (!authentic(retained.token,value.response,value.mac) || value.response.nonce !== request.nonce || value.response.instance !== retained.instance || !['running','stopped','unknown'].includes(value.response.status?.state)) throw Error(); result = value.response.status; finish(); }
      catch { finish(Error('Host ownership could not be verified')); }
    });
    socket.on('close', () => { clearTimeout(timer); if (error || !result) reject(error ?? Error('Host closed without a verified result')); else resolve(result); });
  });
}
/** Missing endpoint/legacy lock is Unknown. No signal or cleanup is based on a PID. */
export async function serviceStatus(directory: string): Promise<ServiceStatus> {
  if (!existsSync(recordPath(directory))) return { state: existsSync(guard(directory)) || existsSync(join(directory,'host.lock')) || existsSync(join(directory,'service.sock')) ? 'unknown' : 'stopped' };
  try { return await call(directory,'status'); } catch { return { state: 'unknown' }; }
}
/** Explicit detached Node launch. Closing the manager cannot stop this service. */
export async function startService(directory: string, entry = new URL('./host-service-child.ts',import.meta.url)): Promise<ServiceStatus> {
  if (Buffer.byteLength(join(directory,'service.sock')) > 100) throw Error('Host directory path is too long for a private service socket');
  const status = await serviceStatus(directory);
  if (status.state === 'running') return status;
  if (status.state !== 'stopped') throw Error('Host ownership is unknown. No process was started or stopped.');
  mkdirSync(directory,{ recursive: true, mode: 0o700 }); privateDirectory(directory); mkdirSync(guard(directory),{ mode: 0o700 });
  const record: Instance = { version: 1, instance: randomBytes(16).toString('hex'), token: randomBytes(32).toString('hex'), socket: join(directory,'service.sock') };
  let written = false;
  try {
    writePrivate(recordPath(directory),record,true); written = true;
    const child = spawn(process.execPath,[fileURLToPath(entry),directory,record.instance],{ detached: true, stdio: 'ignore', env: { PATH: '/usr/bin:/bin', ...(process.env.HOME ? { HOME: process.env.HOME } : {}) } });
    await new Promise<void>((resolve,reject) => { child.once('spawn',resolve); child.once('error',reject); }); child.unref();
  } catch { // This owner has not received a child spawn: its own reservation only.
    if (written) unlinkSync(recordPath(directory)); rmdirSync(guard(directory)); throw Error('Host service could not be launched');
  }
  const end = Date.now()+12000;
  while (Date.now()<end) { const next = await serviceStatus(directory); if (next.state === 'running') return next; if (next.state === 'stopped') throw Error('Host startup failed. Nothing was started successfully.'); await new Promise(r => setTimeout(r,100)); }
  throw Error('Host startup is unconfirmed. Check status before retrying.');
}
/** Stop targets an authenticated observed instance and awaits host teardown. */
export async function stopService(directory: string, expected: string): Promise<ServiceStatus> {
  const result = await call(directory,'stop',expected);
  if (result.state !== 'stopped') return result;
  for (let i = 0; i < 50; i++) {
    const status = await serviceStatus(directory);
    if (status.state === 'stopped') return result;
    await new Promise(r => setTimeout(r,20));
  }
  return { state: 'unknown' };
}

/** Minimal owned local control server. Host close remains the authority for teardown. */
export async function serveHost(directory: string, expected: string, launch: () => Promise<{ close(): Promise<void>; status(): Omit<ServiceStatus,'state' | 'instance'> }>) {
  const retained = instance(directory);
  if (retained.instance !== expected || !existsSync(guard(directory))) throw Error('Service reservation changed');
  let host: Awaited<ReturnType<typeof launch>>;
  try { host = await launch(); }
  catch (error) {
    // host() preserves its lock on incomplete teardown. Never clear that evidence.
    if (!existsSync(join(directory,'host.lock'))) { unlinkSync(recordPath(directory)); rmdirSync(guard(directory)); }
    throw error;
  }
  let stopping: Promise<void> | undefined;
  let closingServer = false;
  const sockets = new Set<Socket>();
  const server = createServer(socket => {
    if (sockets.size >= 16) { socket.destroy(); return; }
    sockets.add(socket); socket.setTimeout(16000,() => socket.destroy());
    socket.on('close',() => sockets.delete(socket)); socket.on('error',() => {});
    let buffer = '', received = false;
    socket.on('data', bytes => {
      if (received) { socket.destroy(); return; }
      if (buffer.length + bytes.length > 8192) { socket.destroy(); return; }
      buffer += bytes.toString(); if (!buffer.includes('\n')) return; received = true;
      void (async () => {
        try {
          const { request, mac } = JSON.parse(buffer.trim());
          if (!authentic(retained.token,request,mac) || request.instance !== retained.instance || !/^[0-9a-f]{32}$/.test(request.nonce) || !['status','stop'].includes(request.action)) throw Error();
          let status: ServiceStatus;
          if (request.action === 'stop') {
            stopping ??= host.close();
            try { await stopping; status = { state: 'stopped', instance: retained.instance }; }
            catch { status = { state: 'unknown', instance: retained.instance }; }
          } else status = { ...host.status(), state: stopping ? 'unknown' : 'running', instance: retained.instance };
          const response = { nonce: request.nonce, instance: retained.instance, status };
          socket.end(JSON.stringify({ response, mac: sign(retained.token,response) })+'\n');
          if (status.state === 'stopped' && !closingServer) {
            closingServer = true;
            server.close(() => { unlinkSync(recordPath(directory)); rmdirSync(guard(directory)); });
            for (const other of sockets) if (other !== socket) other.destroy();
          }
        } catch { socket.destroy(); }
      })();
    });
  });
  try {
    await new Promise<void>((resolve,reject) => { server.once('error',reject); server.listen(retained.socket,resolve); }); chmodSync(retained.socket,0o600);
  } catch (error) { await host.close(); throw error; }
  return server;
}
