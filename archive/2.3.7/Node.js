// Simple WebSocket relay server
// Run with: node server.js

const WebSocket = require('ws');
const wss = new WebSocket.Server({ port: 8765 });

let giga = null;
let browser = null;

wss.on('connection', (ws, req) => {
  const ip = req.socket.remoteAddress;
  console.log("Client connected:", ip);

  ws.on('message', msg => {
    // Identify who is who
    if (msg === 'HELLO_FROM_GIGA') {
      giga = ws;
      console.log("GIGA registered");
      return;
    }
    if (msg === 'HELLO_FROM_BROWSER') {
      browser = ws;
      console.log("Browser registered");
      return;
    }

    // Relay bursts from GIGA → Browser
    if (ws === giga && browser) {
      browser.send(msg);
    }

    // Relay joystick commands Browser → GIGA (optional)
    if (ws === browser && giga) {
      giga.send(msg);
    }
  });

  ws.on('close', () => {
    if (ws === giga) giga = null;
    if (ws === browser) browser = null;
    console.log("Client disconnected:", ip);
  });
});

console.log("WebSocket relay server running on ws://0.0.0.0:8765");