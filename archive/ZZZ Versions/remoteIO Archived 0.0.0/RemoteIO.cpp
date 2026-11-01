#include "RemoteIO.h"

// ============================================================
// Initialization
// ============================================================

void RemoteIO::begin(IPAddress controllerIp,
                     IPAddress consoleIp,
                     uint16_t controllerPort,
                     uint16_t consolePort)
{
    this->controllerIP   = controllerIp;
    this->consoleIP      = consoleIp;
    this->controllerPort = controllerPort;
    this->consolePort    = consolePort;

    udpIn.begin(controllerPort);
    udpOut.begin(consolePort);

    // Clear control arrays
    memset(ctrlBoolArr,   0, sizeof(ctrlBoolArr));
    memset(ctrlIntArr,    0, sizeof(ctrlIntArr));
    memset(ctrlDoubleArr, 0, sizeof(ctrlDoubleArr));
    for (int i = 0; i < 12; i++) ctrlStrArr[i] = "";

    // Clear telemetry arrays
    memset(telemBoolArr,   0, sizeof(telemBoolArr));
    memset(telemIntArr,    0, sizeof(telemIntArr));
    memset(telemDoubleArr, 0, sizeof(telemDoubleArr));
    for (int i = 0; i < 12; i++) telemStrArr[i] = "";

    lastControlSec  = 0;
    lastControlUSec = 0;

    nextSendMicros = micros();
}

// ============================================================
// Time sources
// ============================================================

void RemoteIO::setTimeSourceSeconds(uint32_t (*fn)()) {
    timeSecFn = fn;
}

void RemoteIO::setTimeSourceMicros(uint32_t (*fn)()) {
    timeUSecFn = fn;
}

// ============================================================
// Telemetry rate
// ============================================================

void RemoteIO::setRate(uint8_t hz) {
    if (hz == 0) hz = 1;
    rateHz = hz;
}

// ============================================================
// Poll
// ============================================================

void RemoteIO::poll() {
    parseIncomingPacket();
    sendTelemetryIfDue();
    
}

// ============================================================
// Control getters
// ============================================================

bool RemoteIO::controlBool(uint8_t id) const {
    return (id < 128) ? ctrlBoolArr[id] : false;
}

int32_t RemoteIO::controlInt(uint8_t id) const {
    return (id < 64) ? ctrlIntArr[id] : 0;
}

double RemoteIO::controlDouble(uint8_t id) const {
    return (id < 32) ? ctrlDoubleArr[id] : 0.0;
}

String RemoteIO::controlStr(uint8_t id) const {
    return (id < 12) ? ctrlStrArr[id] : "";
}

// ============================================================
// Telemetry setters
// ============================================================

void RemoteIO::telemBool(uint8_t id, bool value) {
    if (id < 128) telemBoolArr[id] = value;
}

void RemoteIO::telemInt(uint8_t id, int32_t value) {
    if (id < 64) telemIntArr[id] = value;
}

void RemoteIO::telemDouble(uint8_t id, double value) {
    if (id < 32) telemDoubleArr[id] = value;
}

void RemoteIO::telemStr(uint8_t id, const String& value) {
    if (id < 12) telemStrArr[id] = value;
}

// ============================================================
// Bulk accessors (for SD logging)
// ============================================================

const bool* RemoteIO::telemBoolArray() const { return telemBoolArr; }
const int32_t* RemoteIO::telemIntArray() const { return telemIntArr; }
const double* RemoteIO::telemDoubleArray() const { return telemDoubleArr; }
const String* RemoteIO::telemStrArray() const { return telemStrArr; }

const bool* RemoteIO::controlBoolArray() const { return ctrlBoolArr; }
const int32_t* RemoteIO::controlIntArray() const { return ctrlIntArr; }
const double* RemoteIO::controlDoubleArray() const { return ctrlDoubleArr; }
const String* RemoteIO::controlStrArray() const { return ctrlStrArr; }

// ============================================================
// Telemetry scheduling
// ============================================================

void RemoteIO::sendTelemetryIfDue() {
    uint32_t now = micros();
    uint32_t period = 1000000UL / rateHz;

    if ((int32_t)(now - nextSendMicros) >= 0) {
        nextSendMicros += period;
        sendPacketBools();
        sendPacketInts();
        sendPacketDoubles();
        sendPacketStrings();
    }
}

void RemoteIO::sendNow() {
    sendPacketBools();
    sendPacketInts();
    sendPacketDoubles();
    sendPacketStrings();
}

// ============================================================
// Telemetry packet 0 — BOOLS + timestamp
// ============================================================

void RemoteIO::sendPacketBools() {
    uint8_t buf[300];
    int idx = 0;

    buf[idx++] = 0; // packet_type = BOOLS

    uint32_t sec  = timeSecFn  ? timeSecFn()  : (millis() / 1000);
    uint32_t usec = timeUSecFn ? timeUSecFn() : micros();

    memcpy(&buf[idx], &sec, 4);  idx += 4;
    memcpy(&buf[idx], &usec, 4); idx += 4;

    buf[idx++] = 128; // count

    for (int i = 0; i < 128; i++) {
        buf[idx++] = telemBoolArr[i] ? 1 : 0;
    }

    udpOut.beginPacket(consoleIP, consolePort);
    udpOut.write(buf, idx);
    udpOut.endPacket();
}

// ============================================================
// Telemetry packet 1 — INTS
// ============================================================

void RemoteIO::sendPacketInts() {
    uint8_t buf[400];
    int idx = 0;

    buf[idx++] = 1; // packet_type = INTS
    buf[idx++] = 64;

    for (int i = 0; i < 64; i++) {
        int32_t v = telemIntArr[i];
        memcpy(&buf[idx], &v, 4);
        idx += 4;
    }

    udpOut.beginPacket(consoleIP, consolePort);
    udpOut.write(buf, idx);
    udpOut.endPacket();
}

// ============================================================
// Telemetry packet 2 — DOUBLES
// ============================================================

void RemoteIO::sendPacketDoubles() {
    uint8_t buf[400];
    int idx = 0;

    buf[idx++] = 2; // packet_type = DOUBLES
    buf[idx++] = 32;

    for (int i = 0; i < 32; i++) {
        double v = telemDoubleArr[i];
        memcpy(&buf[idx], &v, 8);
        idx += 8;
    }

    udpOut.beginPacket(consoleIP, consolePort);
    udpOut.write(buf, idx);
    udpOut.endPacket();
}

// ============================================================
// Telemetry packet 3 — STRINGS
// ============================================================

void RemoteIO::sendPacketStrings() {
    uint8_t buf[600];
    int idx = 0;

    buf[idx++] = 3; // packet_type = STRINGS
    buf[idx++] = 12;

    for (int i = 0; i < 12; i++) {
        char temp[32];
        memset(temp, 0, 32);
        telemStrArr[i].toCharArray(temp, 32);
        memcpy(&buf[idx], temp, 32);
        idx += 32;
    }

    udpOut.beginPacket(consoleIP, consolePort);
    udpOut.write(buf, idx);
    udpOut.endPacket();
}

// ============================================================
// Incoming control packet parsing
// ============================================================

void RemoteIO::parseIncomingPacket() {
    int packetSize = udpIn.parsePacket();
    if (packetSize <= 0) return;

    uint8_t buf[600];
    int len = udpIn.read(buf, sizeof(buf));
    if (len <= 0) return;

    uint8_t type = buf[0];

    switch (type) {

        // --------------------------------------------------------
        // CONTROL PACKET 0 — BOOLS + timestamp
        // --------------------------------------------------------
        case 0: {
            if (len < 1 + 4 + 4 + 1 + 128) return;

            int idx = 1;

            memcpy(&lastControlSec,  &buf[idx], 4); idx += 4;
            memcpy(&lastControlUSec, &buf[idx], 4); idx += 4;

            uint8_t count = buf[idx++];
            if (count != 128) return;

            for (int i = 0; i < 128; i++) {
                ctrlBoolArr[i] = buf[idx++] ? true : false;
            }
        } break;

        // --------------------------------------------------------
        // CONTROL PACKET 1 — INTS
        // --------------------------------------------------------
        case 1: {
            if (len < 1 + 1 + 64*4) return;

            int idx = 1;
            uint8_t count = buf[idx++];
            if (count != 64) return;

            for (int i = 0; i < 64; i++) {
                int32_t v;
                memcpy(&v, &buf[idx], 4);
                idx += 4;
                ctrlIntArr[i] = v;
            }
        } break;

        // --------------------------------------------------------
        // CONTROL PACKET 2 — DOUBLES
        // --------------------------------------------------------
        case 2: {
            if (len < 1 + 1 + 32*8) return;

            int idx = 1;
            uint8_t count = buf[idx++];
            if (count != 32) return;

            for (int i = 0; i < 32; i++) {
                double v;
                memcpy(&v, &buf[idx], 8);
                idx += 8;
                ctrlDoubleArr[i] = v;
            }
        } break;

        // --------------------------------------------------------
        // CONTROL PACKET 3 — STRINGS
        // --------------------------------------------------------
        case 3: {
            if (len < 1 + 1 + 12*32) return;

            int idx = 1;
            uint8_t count = buf[idx++];
            if (count != 12) return;

            for (int i = 0; i < 12; i++) {
                char temp[33];
                memcpy(temp, &buf[idx], 32);
                temp[32] = 0;
                ctrlStrArr[i] = String(temp);
                idx += 32;
            }
        } break;

        default:
            // Unknown packet type — ignore
            break;
    }
}