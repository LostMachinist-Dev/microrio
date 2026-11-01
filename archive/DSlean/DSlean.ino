#include <Arduino.h>
#include <WiFi.h>
#include <WiFiUdp.h>

// ===============================
// USER CONFIG
// ===============================

// WiFi credentials
const char* WIFI_SSID     = "YOUR_WIFI_NAME";
const char* WIFI_PASSWORD = "REDACTED_WIFI_PASSWORD";

// Static IP configuration
IPAddress LOCAL_IP(192, 168, 0, 50);
IPAddress GATEWAY_IP(192, 168, 0, 1);
IPAddress SUBNET_MASK(255, 255, 255, 0);

// GUI (PC) IP and ports
IPAddress GUI_IP(192, 168, 0, 175);
const uint16_t LOCAL_PORT  = 5005; // GIGA listens here
const uint16_t REMOTE_PORT = 5006; // GUI listens here

// Send rate (ms)
const uint32_t OUTBOUND_PERIOD_MS = 40; // ~25 Hz

// GUI timeout for presence (ms)
const uint32_t GUI_TIMEOUT_MS = 2000;


// ===============================
// DS DATA MODEL
// ===============================

bool   dsBools[32];
double dsDoubles[32];
char   dsStrings[8][64];   // 8 strings, each 64 bytes fixed
bool    ctrlBools[32];
double  ctrlDoubles[32];
char    ctrlStrings[8][64];

//DS test blink
bool booflip = LOW;  
unsigned long previousMillis = 0;  // will store last time LED was updated
const long interval = 1000;  // interval at which to blink (milliseconds)
// ===============================
// DS API (USER-FACING)
// ===============================

void dsWriteBool(int index, bool value) {
    if (index < 0 || index >= 32) return;
    dsBools[index] = value;
}

bool dsReadBool(int index) {
    if (index < 0 || index >= 32) return false;
    return dsBools[index];
}

void dsWrite(int index, double value) {
    if (index < 0 || index >= 32) return;
    dsDoubles[index] = value;
}

double dsRead(int index) {
    if (index < 0 || index >= 32) return 0.0;
    return dsDoubles[index];
}

// Writes a C-string (null-terminated) into fixed 64-byte slot
void dsWriteString(int index, const char* value) {
    if (index < 0 || index >= 8 || value == nullptr) return;

    size_t len = strlen(value);
    if (len > 63) len = 63;        // keep room for explicit null

    // Copy and zero-fill
    memset(dsStrings[index], 0, 64);
    memcpy(dsStrings[index], value, len);
    dsStrings[index][63] = '\0';   // ensure null termination
}

const char* dsReadString(int index) {
    if (index < 0 || index >= 8) return "";
    return dsStrings[index];
}


// ===============================
// DS STATE / HEALTH
// ===============================

enum DsState : uint8_t {
    DS_STATE_DISCONNECTED = 0,
    DS_STATE_CONNECTING   = 1,
    DS_STATE_CONNECTED    = 2,
    DS_STATE_STALE        = 3,
    DS_STATE_ERROR        = 4
};

volatile uint32_t g_frameCounter       = 0; // outbound frames sent
volatile uint32_t g_rxFrameCounterGui  = 0; // last inbound frameId from GUI
volatile uint64_t g_timestampNetworkUs = 0; // local notion of "network" time
volatile DsState  g_dsState            = DS_STATE_DISCONNECTED;

// GUI presence tracking
uint32_t g_lastInboundMs = 0;

// Heartbeat
uint32_t g_heartbeat = 0;


// ===============================
// UDP / NETWORK
// ===============================

WiFiUDP udp;

// We now send 4 packets per frame, each with a small header.
const uint8_t PACKET_COUNT_PER_FRAME = 4;

// Packet header layout (per UDP packet):
//   uint32_t frameId
//   uint32_t rxFrameIdFromGui
//   uint64_t timestampNetworkMicros
//   uint8_t  dsState
//   uint8_t  packetIndex   (0..3)
//   uint8_t  packetCount   (=4)
//   uint8_t  reserved
//
// Then payload depends on packetIndex.
//
// Packet 0 payload:
//   uint32_t boolBits
//   double   dsDoubles[0..15]
//
// Packet 1 payload:
//   double   dsDoubles[16..31]
//
// Packet 2 payload:
//   char dsStrings[0..3][64]
//
// Packet 3 payload:
//   char dsStrings[4..7][64]

const size_t HEADER_SIZE = 4 + 4 + 8 + 1 + 1 + 1 + 1; // 20 bytes

// Payload sizes
const size_t P0_PAYLOAD_SIZE = 4 + (16 * 8);      // 4 + 128 = 132
const size_t P1_PAYLOAD_SIZE = (16 * 8);          // 128
const size_t P2_PAYLOAD_SIZE = 4 * 64;            // 256
const size_t P3_PAYLOAD_SIZE = 4 * 64;            // 256

const size_t P0_PACKET_SIZE = HEADER_SIZE + P0_PAYLOAD_SIZE; // 152
const size_t P1_PACKET_SIZE = HEADER_SIZE + P1_PAYLOAD_SIZE; // 148
const size_t P2_PACKET_SIZE = HEADER_SIZE + P2_PAYLOAD_SIZE; // 276
const size_t P3_PACKET_SIZE = HEADER_SIZE + P3_PAYLOAD_SIZE; // 276

// Max needed for any outbound packet
const size_t MAX_OUTBOUND_PACKET_SIZE = HEADER_SIZE + P2_PAYLOAD_SIZE;


// ===============================
// BINARY HELPERS (LITTLE-ENDIAN)
// ===============================

void writeUint32(uint8_t* buf, int& offset, uint32_t value) {
    buf[offset++] = (uint8_t)(value & 0xFF);
    buf[offset++] = (uint8_t)((value >> 8) & 0xFF);
    buf[offset++] = (uint8_t)((value >> 16) & 0xFF);
    buf[offset++] = (uint8_t)((value >> 24) & 0xFF);
}

uint32_t readUint32(const uint8_t* buf, int& offset) {
    uint32_t b0 = buf[offset++];
    uint32_t b1 = buf[offset++];
    uint32_t b2 = buf[offset++];
    uint32_t b3 = buf[offset++];
    return (b0) | (b1 << 8) | (b2 << 16) | (b3 << 24);
}

void writeUint64(uint8_t* buf, int& offset, uint64_t value) {
    buf[offset++] = (uint8_t)(value & 0xFF);
    buf[offset++] = (uint8_t)((value >> 8) & 0xFF);
    buf[offset++] = (uint8_t)((value >> 16) & 0xFF);
    buf[offset++] = (uint8_t)((value >> 24) & 0xFF);
    buf[offset++] = (uint8_t)((value >> 32) & 0xFF);
    buf[offset++] = (uint8_t)((value >> 40) & 0xFF);
    buf[offset++] = (uint8_t)((value >> 48) & 0xFF);
    buf[offset++] = (uint8_t)((value >> 56) & 0xFF);
}

uint64_t readUint64(const uint8_t* buf, int& offset) {
    uint64_t b0 = buf[offset++];
    uint64_t b1 = buf[offset++];
    uint64_t b2 = buf[offset++];
    uint64_t b3 = buf[offset++];
    uint64_t b4 = buf[offset++];
    uint64_t b5 = buf[offset++];
    uint64_t b6 = buf[offset++];
    uint64_t b7 = buf[offset++];
    return (b0)
         | (b1 << 8)
         | (b2 << 16)
         | (b3 << 24)
         | (b4 << 32)
         | (b5 << 40)
         | (b6 << 48)
         | (b7 << 56);
}

void writeDouble(uint8_t* buf, int& offset, double value) {
    uint8_t temp[8];
    memcpy(temp, &value, 8);
    for (int i = 0; i < 8; i++) {
        buf[offset++] = temp[i];
    }
}

double readDouble(const uint8_t* buf, int& offset) {
    uint8_t temp[8];
    for (int i = 0; i < 8; i++) {
        temp[i] = buf[offset++];
    }
    double value;
    memcpy(&value, temp, 8);
    return value;
}


// ===============================
// PACKING HELPERS
// ===============================

// Pack dsBools[32] into 4 bytes (LSB of each bit)
uint32_t packBoolBits() {
    uint32_t bits = 0;
    for (int i = 0; i < 32; i++) {
        if (dsBools[i]) {
            bits |= (1UL << i);
        }
    }
    return bits;
}

void unpackBoolBits(uint32_t bits) {
    for (int i = 0; i < 32; i++) {
        dsBools[i] = (bits & (1UL << i)) != 0;
    }
}


// ===============================
// OUTBOUND PACKETS (GIGA → GUI)
// ===============================

void writeCommonHeader(uint8_t* buffer, int& offset, uint8_t packetIndex) {
    // frameId
    writeUint32(buffer, offset, g_frameCounter);
    // rxFrameIdFromGui
    writeUint32(buffer, offset, g_rxFrameCounterGui);

    // For now, use local micros() as "network" time placeholder
    g_timestampNetworkUs = (uint64_t)micros();
    writeUint64(buffer, offset, g_timestampNetworkUs);

    buffer[offset++] = (uint8_t)g_dsState;          // dsState
    buffer[offset++] = packetIndex;                 // packetIndex
    buffer[offset++] = PACKET_COUNT_PER_FRAME;      // packetCount
    buffer[offset++] = 0;                           // reserved
}
                                                    //_____HALFWAY_______________________
void buildAndSendPacket0() {
    uint8_t buffer[P0_PACKET_SIZE];
    int offset = 0;

    writeCommonHeader(buffer, offset, 0);

    // Payload: boolBits + first 16 doubles
    uint32_t boolBits = packBoolBits();
    writeUint32(buffer, offset, boolBits);

    for (int i = 0; i < 16; i++) {
        writeDouble(buffer, offset, dsDoubles[i]);
    }

    udp.beginPacket(GUI_IP, REMOTE_PORT);
    udp.write(buffer, offset);
    udp.endPacket();
}

void buildAndSendPacket1() {
    uint8_t buffer[P1_PACKET_SIZE];
    int offset = 0;

    writeCommonHeader(buffer, offset, 1);

    // Payload: doubles[16..31]
    for (int i = 16; i < 32; i++) {
        writeDouble(buffer, offset, dsDoubles[i]);
    }

    udp.beginPacket(GUI_IP, REMOTE_PORT);
    udp.write(buffer, offset);
    udp.endPacket();
}

void buildAndSendPacket2() {
    uint8_t buffer[P2_PACKET_SIZE];
    int offset = 0;

    writeCommonHeader(buffer, offset, 2);

    // Payload: strings[0..3]
    for (int i = 0; i < 4; i++) {
        dsStrings[i][63] = '\0';
        memcpy(&buffer[offset], dsStrings[i], 64);
        offset += 64;
    }

    udp.beginPacket(GUI_IP, REMOTE_PORT);
    udp.write(buffer, offset);
    udp.endPacket();
}

void buildAndSendPacket3() {
    uint8_t buffer[P3_PACKET_SIZE];
    int offset = 0;

    writeCommonHeader(buffer, offset, 3);

    // Payload: strings[4..7]
    for (int i = 4; i < 8; i++) {
        dsStrings[i][63] = '\0';
        memcpy(&buffer[offset], dsStrings[i], 64);
        offset += 64;
    }

    udp.beginPacket(GUI_IP, REMOTE_PORT);
    udp.write(buffer, offset);
    udp.endPacket();
}

void sendOutboundFrame() {
    buildAndSendPacket0();
    buildAndSendPacket1();
    buildAndSendPacket2();
    buildAndSendPacket3();

    g_frameCounter++;
}


// ===============================
// INBOUND PACKETS (GUI → GIGA)
// ===============================
//
// Same multi-packet structure expected from GUI.
// For now, we implement a simple handler that applies
// each packet as it arrives, without strict reassembly.
// Later, Node/GUI can send matching packets.

void handleInboundPacket(const uint8_t* buffer, size_t length) {
    if (length < HEADER_SIZE) {
        return;
    }

    int offset = 0;

    uint32_t frameIdFromGui = readUint32(buffer, offset);
    uint32_t ackFrameId     = readUint32(buffer, offset); // unused for now
    uint64_t guiTimestampUs = readUint64(buffer, offset); // unused for now

    uint8_t dsStateFromGui  = buffer[offset++];
    uint8_t packetIndex     = buffer[offset++];
    uint8_t packetCount     = buffer[offset++];
    offset++; // reserved

    g_rxFrameCounterGui = frameIdFromGui;
    g_lastInboundMs = millis();

    // Minimal safety
    if (packetCount != PACKET_COUNT_PER_FRAME || packetIndex > 3) {
        return;
    }

    switch (packetIndex) {
        case 0: {
            if (length < HEADER_SIZE + P0_PAYLOAD_SIZE) return;
            uint32_t boolBits = readUint32(buffer, offset);
            for (int i = 0; i < 32; i++) {
                ctrlBools[i] = (boolBits & (1UL << i)) != 0;
            }

            for (int i = 0; i < 16; i++) {
                ctrlDoubles[i] = readDouble(buffer, offset);
            }
            break;
        }
        case 1: {
            if (length < HEADER_SIZE + P1_PAYLOAD_SIZE) return;
            for (int i = 16; i < 32; i++) {
                ctrlDoubles[i] = readDouble(buffer, offset);
            }

            break;
        }
        case 2: {
            if (length < HEADER_SIZE + P2_PAYLOAD_SIZE) return;
            for (int i = 0; i < 4; i++) {
                memcpy(ctrlStrings[i], &buffer[offset], 64);
                ctrlStrings[i][63] = '\0';
                offset += 64;
            }
            break;
        }
        case 3: {
            if (length < HEADER_SIZE + P3_PAYLOAD_SIZE) return;
            for (int i = 4; i < 8; i++) {
                memcpy(ctrlStrings[i], &buffer[offset], 64);
                ctrlStrings[i][63] = '\0';
                offset += 64;
            }
            break;
        }
        default:
            break;
    }

    (void)dsStateFromGui;
    (void)guiTimestampUs;
    (void)ackFrameId;
}


// ===============================
// LED / STATE MANAGEMENT
// ===============================

void updateDsState() {
    uint8_t wifiStatus = WiFi.status();

    if (wifiStatus != WL_CONNECTED) {
        g_dsState = DS_STATE_DISCONNECTED;
        return;
    }

    uint32_t now = millis();
    if (now - g_lastInboundMs > GUI_TIMEOUT_MS) {
        g_dsState = DS_STATE_STALE;
    } else {
        g_dsState = DS_STATE_CONNECTED;
    }
}

void updateLeds() {
    // Blue LED: WiFi status
    if (WiFi.status() == WL_CONNECTED) {
        digitalWrite(LED_BLUE, HIGH);  // OFF (assuming active LOW)
    } else {
        digitalWrite(LED_BLUE, LOW);   // ON while not connected
    }

    // Green LED: GUI presence
    uint32_t now = millis();
    if (now - g_lastInboundMs < GUI_TIMEOUT_MS) {
        digitalWrite(LED_GREEN, LOW);  // ON when GUI seen recently
    } else {
        digitalWrite(LED_GREEN, HIGH); // OFF when GUI missing
    }
}


// ===============================
// WIFI INITIALIZATION
// ===============================

void connectWiFi() {
    g_dsState = DS_STATE_CONNECTING;

    WiFi.end();
    delay(1500);

    WiFi.config(LOCAL_IP, GATEWAY_IP, SUBNET_MASK);
    WiFi.begin(WIFI_SSID, WIFI_PASSWORD);

    while (WiFi.status() != WL_CONNECTED) {
        delay(250);
    }

    g_dsState = DS_STATE_CONNECTED;
}


// ===============================
// SETUP
// ===============================

void setup() {
    // No Serial.begin() to avoid USB/CDC issues

    // LEDs
    pinMode(LED_BLUE, OUTPUT);
    pinMode(LED_GREEN, OUTPUT);

    digitalWrite(LED_BLUE, LOW);   // ON while connecting
    digitalWrite(LED_GREEN, HIGH); // Assume GUI missing until proven otherwise

    // Initialize DS arrays with some defaults
    for (int i = 0; i < 32; i++) {
        dsBools[i] = false;
        dsDoubles[i] = 0.0;
    }
    for (int i = 0; i < 8; i++) {
        memset(dsStrings[i], 0, 64);
        strncpy(dsStrings[i], "INIT", 63);
        dsStrings[i][63] = '\0';
    }

    connectWiFi();
    udp.begin(LOCAL_PORT);

    g_lastInboundMs = 0;
    g_frameCounter = 0;
    g_rxFrameCounterGui = 0;

}


// ===============================
// LOOP
// ===============================

void loop() {
    // ----------------------------
    // RECEIVE: GUI → GIGA
    // ----------------------------
    int packetSize = udp.parsePacket();
    if (packetSize > 0) {
        // Max packet we expect is P2/P3 = 276 bytes; 512 is safe
        uint8_t inboundBuffer[512];
        int len = udp.read(inboundBuffer, sizeof(inboundBuffer));
        if (len > 0) {
            handleInboundPacket(inboundBuffer, (size_t)len);
        }
    }

    // ----------------------------
    // UPDATE STATE / LEDS
    // ----------------------------
    updateDsState();
    updateLeds();

    // ----------------------------
    // UPDATE OUTBOUND DATA (EXAMPLE)
    // ----------------------------
    // Example: sample first 8 analog channels into dsDoubles[0..7]
    for (int i = 0; i < 8; i++) {
        int raw = analogRead(i);
        dsWrite(i, (double)raw);
    }

    // Example: heartbeat in a double channel
    g_heartbeat++;
    dsWrite(31, (double)g_heartbeat);

    // ----------------------------
    // SEND OUTBOUND FRAME
    // ----------------------------
    static uint32_t lastSendMs = 0;
    uint32_t nowMs = millis();

    if (nowMs - lastSendMs >= OUTBOUND_PERIOD_MS) {
        lastSendMs = nowMs;
        sendOutboundFrame();
    }
    unsigned long currentMillis = millis();
    if (currentMillis - previousMillis >= interval) {
    previousMillis = currentMillis;

    // if the LED is off turn it on and vice-versa:
    if (booflip == LOW) {
      booflip = HIGH;
    } else {
      booflip = LOW;
    }

    // set the LED with the booflip of the variable:
    dsWriteBool(6, booflip);
  }
}