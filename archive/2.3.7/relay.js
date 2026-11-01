// relay.js - UDP (GIGA) <-> WebSocket (browser)
// Run: npm install ws
// Then: node relay.js

const dgram = require('dgram');
const WebSocket = require('ws');

const UDP_PORT = 5801;     // GIGA -> PC telemetry
const WS_PORT  = 8765;     // PC -> browser WebSocket

const udp = dgram.createSocket('udp4');
const wss = new WebSocket.Server({ port: WS_PORT });

let lastSender = null;     // Remember GIGA's address/port

udp.on('message', (msg, rinfo) => {
  lastSender = rinfo;
  // Broadcast binary burst to all browser clients
  wss.clients.forEach(ws => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(msg);
    }
  });
});

udp.on('listening', () => {
  const addr = udp.address();
  console.log(`UDP listening on ${addr.address}:${addr.port}`);
});

udp.bind(UDP_PORT);

wss.on('connection', ws => {
  console.log('Browser WebSocket connected');
  ws.on('close', () => console.log('Browser WebSocket disconnected'));
});

console.log(`WebSocket relay server on ws://0.0.0.0:${WS_PORT}`);