// ------------------------------------------------------------
// joystick-service.js
// HID-agnostic, multi-controller, bulletproof
// ------------------------------------------------------------

const HID = require("node-hid");
const fs = require("fs");
const WebSocket = require("ws");

// ------------------------------------------------------------
// CONFIG
// ------------------------------------------------------------
const POLL_RATE = 25; // Hz
const MAPPING_FILE = "mapping.json";

// ------------------------------------------------------------
// STATE
// ------------------------------------------------------------
let controllers = {};        // { uniqueID: { hid, info, mapping } }
let wsSetupClients = [];     // joySetup.html
let wsRuntime = null;        // server.js
let mappingDB = {};          // entire mapping.json contents

// ------------------------------------------------------------
// SAFE LOAD MAPPING.JSON
// ------------------------------------------------------------
function safeLoadMapping() {
    try {
        if (!fs.existsSync(MAPPING_FILE)) {
            console.log("mapping.json missing — creating empty.");
            fs.writeFileSync(MAPPING_FILE, JSON.stringify({ controllers: {} }, null, 2));
            return { controllers: {} };
        }

        const txt = fs.readFileSync(MAPPING_FILE, "utf8").trim();
        if (!txt) {
            console.log("mapping.json empty — using {}.");
            return { controllers: {} };
        }

        const parsed = JSON.parse(txt);
        if (!parsed.controllers) parsed.controllers = {};
        return parsed;

    } catch (e) {
        console.log("Error loading mapping.json:", e.message);
        return { controllers: {} };
    }
}

mappingDB = safeLoadMapping();

// Hot reload
fs.watch(MAPPING_FILE, () => {
    console.log("mapping.json changed, reloading...");
    mappingDB = safeLoadMapping();
});

// ------------------------------------------------------------
// UNIQUE CONTROLLER ID
// ------------------------------------------------------------
function makeID(dev) {
    const vid = dev.vendorId?.toString(16).padStart(4, "0");
    const pid = dev.productId?.toString(16).padStart(4, "0");
    const sn = dev.serialNumber || "NOSN";
    return `${vid}-${pid}-${sn}`;
}

// ------------------------------------------------------------
// FIND ALL JOYSTICKS
// ------------------------------------------------------------
function findAllJoysticks() {
    return HID.devices().filter(d =>
        d.usagePage === 1 &&
        (d.usage === 4 || d.usage === 5 || d.usage === 8)
    );
}

// ------------------------------------------------------------
// CONNECT ALL JOYSTICKS
// ------------------------------------------------------------
function connectControllers() {
    const devices = findAllJoysticks();

    devices.forEach(dev => {
        const id = makeID(dev);

        if (controllers[id]) return; // already connected

        try {
            const hid = new HID.HID(dev.path);
            console.log("Connected:", dev.product, "ID:", id);

            controllers[id] = {
                hid,
                info: dev,
                mapping: mappingDB.controllers[id]?.mapping || {}
            };

            // If mapping entry doesn't exist, create it
            if (!mappingDB.controllers[id]) {
                mappingDB.controllers[id] = {
                    identity: {
                        vendorId: dev.vendorId,
                        productId: dev.productId,
                        serialNumber: dev.serialNumber || null,
                        manufacturer: dev.manufacturer,
                        product: dev.product
                    },
                    mapping: {}
                };
                fs.writeFileSync(MAPPING_FILE, JSON.stringify(mappingDB, null, 2));
            }

        } catch (e) {
            console.log("Failed to connect HID:", e.message);
        }
    });
}

// Initial scan
connectControllers();

// Rescan every 5 seconds for hot-plug
setInterval(connectControllers, 5000);

// ------------------------------------------------------------
// APPLY MAPPING
// ------------------------------------------------------------
function applyMapping(raw, mapping) {
    const result = { axes: {}, buttons: {}, hats: {} };

    for (const chStr in mapping) {
        const cfg = mapping[chStr];
        const ch = Number(chStr);
        const val = raw[ch];

        if (val === undefined || !cfg) continue;

        try {
            if (cfg.type === "axis") {
                result.axes[cfg.target] = val;

            } else if (cfg.type === "bitmask") {
                for (const bitStr in cfg.buttonBits) {
                    const bit = Number(bitStr);
                    const name = cfg.buttonBits[bitStr];
                    result.buttons[name] = (val >> bit) & 1;
                }

            } else if (cfg.type === "mixed") {
                const hatVal = (val >> cfg.hatStart) & 0x0F;
                result.hats[cfg.hatTarget] = hatVal;

                for (const bitStr in cfg.buttonBits) {
                    const bit = Number(bitStr);
                    const name = cfg.buttonBits[bitStr];
                    result.buttons[name] = (val >> bit) & 1;
                }
            }

        } catch (e) {
            console.log("Mapping error:", e.message);
        }
    }

    return result;
}

// ------------------------------------------------------------
// joySetup.html WEBSOCKET SERVER
// ------------------------------------------------------------
const wssSetup = new WebSocket.Server({ port: 9001 });

wssSetup.on("connection", ws => {
    console.log("joySetup connected");
    wsSetupClients.push(ws);

    ws.on("close", () => {
        wsSetupClients = wsSetupClients.filter(c => c !== ws);
    });

    ws.on("message", msg => {
        try {
            const data = JSON.parse(msg);

            if (data.type === "saveMapping" && data.controllerID) {
                mappingDB.controllers[data.controllerID].mapping = data.mapping;
                fs.writeFileSync(MAPPING_FILE, JSON.stringify(mappingDB, null, 2));
                console.log("mapping.json updated for", data.controllerID);
            }

        } catch (e) {
            console.log("Bad joySetup message:", e.message);
        }
    });
});

// ------------------------------------------------------------
// CONNECT TO server.js (runtime)
// ------------------------------------------------------------
function connectRuntime() {
    const ws = new WebSocket("ws://localhost:9002");

    ws.on("open", () => {
        console.log("Connected to server.js");
        wsRuntime = ws;
    });

    ws.on("close", () => {
        console.log("Disconnected from server.js, retrying...");
        wsRuntime = null;
        setTimeout(connectRuntime, 2000);
    });

    ws.on("error", () => {});
}

connectRuntime();

// ------------------------------------------------------------
// MAIN POLLING LOOP
// ------------------------------------------------------------
setInterval(() => {
    for (const id in controllers) {
        const c = controllers[id];

        try {
            const data = c.hid.readSync();
            const raw = Array.from(data);

            // Send RAW to joySetup
            wsSetupClients.forEach(ws => {
                if (ws.readyState === WebSocket.OPEN) {
                    ws.send(JSON.stringify({
                        type: "raw",
                        controllerID: id,
                        raw
                    }));
                }
            });

            // Apply mapping
            const mapped = applyMapping(raw, mappingDB.controllers[id].mapping);

            // Send MAPPED to server.js
            if (wsRuntime && wsRuntime.readyState === WebSocket.OPEN) {
                wsRuntime.send(JSON.stringify({
                    type: "mapped",
                    controllerID: id,
                    raw,
                    mapped
                }));
            }

        } catch (e) {
            console.log("HID read error for", id, e.message);
            try { c.hid.close(); } catch {}
            delete controllers[id];
        }
    }

}, 1000 / POLL_RATE);