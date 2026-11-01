const HID = require("node-hid");

// --------------------------------------------------
// 1. Find a joystick HID device
// --------------------------------------------------
const devices = HID.devices();
let deviceInfo = devices.find(d =>
    d.usagePage === 1 && (d.usage === 4 || d.usage === 5)
);

if (!deviceInfo) {
    console.log("No joystick found.");
    process.exit(1);
}

console.log("Using device:", deviceInfo.product);

// Build a unique ID object
const joystickID = {
    vendorId: deviceInfo.vendorId,
    productId: deviceInfo.productId,
    product: deviceInfo.product,
    manufacturer: deviceInfo.manufacturer,
    serialNumber: deviceInfo.serialNumber || null
};

console.log("Joystick ID:", joystickID);

// --------------------------------------------------
// 2. Open HID device
// --------------------------------------------------
const device = new HID.HID(deviceInfo.path);

// --------------------------------------------------
// 3. Shared state (latest HID data)
// --------------------------------------------------
let joyRaw = new Array(64).fill(0);   // raw bytes
let joyMapped = {};                   // placeholder for mapped values

// --------------------------------------------------
// 4. HID event handler (FAST)
//    Updates joyRaw only
// --------------------------------------------------
device.on("data", (data) => {
    const r = new Uint8Array(data);
    for (let i = 0; i < 64; i++) {
        joyRaw[i] = r[i];
    }
});

device.on("error", (err) => {
    console.log("HID ERROR:", err);
});

// --------------------------------------------------
// 5. 25 Hz poller
//    Reads joyRaw → prints clean output
//    (joyMapped will be filled later by mapping logic)
// --------------------------------------------------
setInterval(() => {
    console.clear();
    console.log("25 Hz JOYSTICK RAW DATA");
    console.log("------------------------");
    console.log("Joystick ID:", joystickID);

    // Print first 16 channels for readability
    for (let i = 0; i < 16; i++) {
        console.log(`Raw[${i}] = ${joyRaw[i]}`);
    }

    console.log("\nMapped (placeholder):", joyMapped);

}, 40);   // 25 Hz (1000ms / 25 = 40ms)

// --------------------------------------------------
// 6. WebSocket server to send joyRaw + joystickID
// --------------------------------------------------
const WebSocket = require("ws");
const wss = new WebSocket.Server({ port: 8081 });

wss.on("connection", (client) => {
    console.log("joySetup.html connected");

    client.send(JSON.stringify({
        type: "hello",
        joystickID
    }));
});

// Broadcast joyRaw at 25 Hz
setInterval(() => {
    const packet = JSON.stringify({
        type: "joyRaw",
        joyRaw,
        joystickID
    });

    wss.clients.forEach(client => {
        if (client.readyState === WebSocket.OPEN) {
            client.send(packet);
        }
    });
}, 40); // 25 Hz