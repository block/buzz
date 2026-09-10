/** Private stdio transport only: no subprocesses, executable selection or env grants. */
import { connect } from 'node:net';
const [path, capability] = process.argv.slice(2);
if (!path || !capability || process.argv.length !== 4) process.exit(2);
const socket = connect(path);
const timer = setTimeout(() => process.exit(2), 5000);
socket.once('connect', () => {
  clearTimeout(timer);
  socket.write(JSON.stringify({ capability, pid: process.pid }) + '\n');
  process.stdin.pipe(socket);
  socket.pipe(process.stdout);
});
// Upstream starts this shim in a separate process group. It never spawns children;
// EOF, disconnect and TERM all terminate it, independently of upstream lifetime.
const exit = () => { socket.destroy(); process.exit(0); };
socket.on('error', exit); socket.on('close', exit);
process.stdin.on('end', exit); process.stdout.on('error', exit);
process.on('SIGTERM', exit); process.on('SIGINT', exit);
