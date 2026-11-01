// =======================================
// UDP GUI TEST RECEIVER
// =======================================

const dgram = require("dgram");
const socket = dgram.createSocket("udp4");

const LISTEN_PORT = 5006;   // Must match Arduino REMOTE_PORT
let packetCount = 0;

socket.on("message", (msg, rinfo) => {
    packetCount++;

    console.clear();
    console.log("=======================================");
    console.log(" UDP GUI TEST RECEIVER");
    console.log("=======================================");
    console.log("From:", rinfo.address, ":", rinfo.port);
    console.log("Packet:", packetCount);
    console.log("Time:", new Date().toLocaleTimeString());
    console.log("---------------------------------------");

    try {
        const data = JSON.parse(msg.toString());

        console.log("BOOLS:", data.bools);
        console.log("DOUBLES:", data.doubles);
        console.log("FLOATS:", data.floats);
        console.log("INTS:", data.ints);
        console.log("STRINGS:", data.strings);

        console.log("---------------------------------------");
        console.log("Analog Inputs (floats):");
        for (let i = 0; i < 8; i++) {
            console.log(`AIN${i}:`, data.floats[i]);
        }

    } catch (err) {
        console.log("JSON PARSE ERROR:", err);
        console.log("RAW MESSAGE:", msg.toString());
    }
});

socket.bind(LISTEN_PORT, () => {
    console.log("=======================================");
    console.log(" UDP GUI TEST RECEIVER STARTED");
    console.log(" Listening on port", LISTEN_PORT);
    console.log("=======================================");
});