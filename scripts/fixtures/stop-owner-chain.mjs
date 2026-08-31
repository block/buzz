// Isolated protocol fixture ONLY: accepts NIP-42 without verification and serves
// canned OpenAI tool calls. No real relay, provider, credentials or paid work.
// Run only via the ignored native selected_generation_process_chain test.
import http from 'node:http';
import { createHash } from 'node:crypto';
import { writeFileSync } from 'node:fs';

function frame(value) {
  const body = Buffer.from(JSON.stringify(value));
  const header = body.length < 126 ? Buffer.from([0x81, body.length]) : Buffer.from([0x81, 126, body.length >> 8, body.length & 255]);
  return Buffer.concat([header, body]);
}
const server = http.createServer(async (req, res) => {
  let body = '';
  for await (const chunk of req) {
    body += chunk;
    if (body.length > 4_000_000) { res.writeHead(413).end(); return; }
  }
  let result = [];
  if (req.url.includes('chat/completions')) {
    const parsed = JSON.parse(body);
    const tool = parsed.tools?.find(t => t.function?.name.endsWith('__shell'))?.function.name;
    if (!tool) { res.writeHead(400).end('shell tool missing'); return; }
    result = {id:'fixture', object:'chat.completion', model:'fixture', choices:[{
      index:0, message:{role:'assistant', content:null, tool_calls:[{
        id:'fixture-shell', type:'function', function:{name:tool, arguments:JSON.stringify({
          command:'echo $$ > shell.pid; echo $PPID > mcp.pid; sleep 600 & echo $! > grandchild.pid; wait',
          timeout_ms:600000
        })}
      }]}, finish_reason:'tool_calls'
    }]};
  } else if (req.url === '/events') {
    result = {accepted:true};
  }
  res.writeHead(200, {'content-type':'application/json'}).end(JSON.stringify(result));
});
server.on('upgrade', (req, socket) => {
  const accept = createHash('sha1').update(req.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
  socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
  socket.write(frame(['AUTH','stop-chain-fixture']));
  let buffer = Buffer.alloc(0);
  socket.on('error', () => {});
  socket.on('data', data => {
    buffer = Buffer.concat([buffer, data]);
    while (buffer.length >= 2) {
      const opcode = buffer[0] & 15;
      let len = buffer[1] & 127;
      let offset = 2;
      if (len === 126) { if (buffer.length < 4) return; len = buffer.readUInt16BE(2); offset = 4; }
      if (len === 127 || !(buffer[1] & 128)) { socket.destroy(); return; }
      if (buffer.length < offset + 4 + len) return;
      const mask = buffer.subarray(offset, offset + 4); offset += 4;
      const payload = Buffer.from(buffer.subarray(offset, offset + len));
      buffer = buffer.subarray(offset + len);
      for (let i = 0; i < len; i++) payload[i] ^= mask[i % 4];
      if (opcode === 8) { socket.end(Buffer.from([0x88, 0])); return; }
      if (opcode !== 1) continue;
      const msg = JSON.parse(payload.toString());
      if (msg[0] === 'AUTH' || msg[0] === 'EVENT') socket.write(frame(['OK', msg[1].id, true, 'fixture']));
      if (msg[0] === 'REQ') socket.write(frame(['EOSE',msg[1]]));
    }
  });
});
server.listen(0, '127.0.0.1', () => writeFileSync(process.argv[2], String(server.address().port)));
