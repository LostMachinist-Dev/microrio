// ------------------------------------------------------------
// server.js — runtime router + controlInt builder
// ------------------------------------------------------------

const fs = require("fs");
const WebSocket = require("ws");

// ------------------------------------------------------------
// CONFIG
// ------------------------------------------------------------
const CONFIG_FILE = "config.json";
let config = loadConfig();

// ------------------------------------------------------------
// SAFE LOAD CONFIG.JSON
// ------------------------------------------------------------
function loadConfig() {
    try {
        if (!fs.existsSync(CONFIG_FILE)) {
            console.log("config.json missing — creating default.");
            const def = { controllers: {}, controlIntSize: 32 };
            fs.writeFileSync(CONFIG_FILE, JSON.stringify(def, null, 2));
            return def;
        }

        const txt = fs.readFileSync(CONFIG_FILE, "utf8").trim();
        if (!txt) return { controllers: {}, controlIntSize: 32 };

        const parsed = JSON.parse(txt);
        if (!parsed.controllers) parsed.controllers = {};
        if (!parsed.controlIntSize) parsed.controlIntSize = 32;
        return parsed;

    } catch (e) {
        console.log("Error loading config.json:", e.message);
        return { controllers: {}, controlIntSize: 32 };
    }
}

// Hot reload config.json
fs.watch(CONFIG_FILE, () => {
    console.log("config.json changed, reloading...");
    config = loadConfig();
});

// ------------------------------------------------------------
// WEBSOCKET SERVER FOR index.html
// ------------------------------------------------------------
const wssBrowser = new WebSocket.Server({ port: 8080 });
let browserClients = [];

wssBrowser.on("connection", ws => {
    console.log("index.html connected");
    browserClients.push(ws);

    ws.on("close", () => {
        browserClients = browserClients.filter(c => c !== ws);
    });
});

// ------------------------------------------------------------
// CONNECT TO joystick-service.js
// ------------------------------------------------------------
let wsJoystick = null;
let lastHeartbeat = 0;

function startJoystickServiceListener() {
    const wssJoystick = new WebSocket.Server({ port: 9002 });

    console.log("server.js listening for joystick-service.js on ws://localhost:9002");

    wssJoystick.on("connection", ws => {
        console.log("joystick-service.js connected");
        wsJoystick = ws;

        // Track heartbeat
        lastHeartbeat = Date.now();

        // Handle incoming messages
        ws.on("message", msg => {
            lastHeartbeat = Date.now();
            
            try {
                const data = JSON.parse(msg);

                console.log("FROM JS-SERVICE:", data);

                // You will handle mapped packets here
                if (data.type === "mapped") {
                    // Example: forward to browser or Arduino
                    // handleMappedPacket(data);
                }

            } catch (e) {
                console.log("Bad message from joystick-service.js:", e.message);
            }
            
        });

        // Handle disconnect
        ws.on("close", () => {
            console.log("joystick-service.js disconnected");
            wsJoystick = null;
        });

        ws.on("error", err => {
            console.log("joystick-service.js socket error:", err.message);
        });
    });
}

startJoystickServiceListener();

// ------------------------------------------------------------
// BUILD controlInt[] FROM MAPPED DATA
// ------------------------------------------------------------
function buildControlInt(controllerID, mapped) {
    const cfg = config.controllers[controllerID];
    const size = config.controlIntSize || 32;
    const out = new Array(size).fill(0);

    if (!cfg) return out;

    // Axes
    for (const axisName in mapped.axes) {
        const idx = cfg.axes?.[axisName];
        if (idx !== undefined) out[idx] = mapped.axes[axisName];
    }

    // Buttons
    for (const btnName in mapped.buttons) {
        const idx = cfg.buttons?.[btnName];
        if (idx !== undefined) out[idx] = mapped.buttons[btnName];
    }

    // Hats
    for (const hatName in mapped.hats) {
        const idx = cfg.hats?.[hatName];
        if (idx !== undefined) out[idx] = mapped.hats[hatName];
    }

    return out;
}

// ------------------------------------------------------------
// SEND TO BROWSER
// ------------------------------------------------------------
function broadcastToBrowser(obj) {
    const msg = JSON.stringify(obj);
    browserClients.forEach(ws => {
        if (ws.readyState === WebSocket.OPEN) ws.send(msg);
    });
}

// ------------------------------------------------------------
// HEARTBEAT CHECK
// ------------------------------------------------------------
setInterval(() => {
    const alive = (Date.now() - lastHeartbeat) < 2000;

    broadcastToBrowser({
        type: "joystickServiceStatus",
        alive
    });

}, 500);