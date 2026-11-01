#include <Arduino.h>
#include <WiFi.h>
#include <WiFiUdp.h>
#include <Servo.h>
Servo ctrlServo;
const int SERVO_PIN = D2;
double joyval = 0;
unsigned long startTime;
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
char   dsStrings[8][64];

// CONTROL ARRAYS (GUI → GIGA)
bool    ctrlBools[32];
double  ctrlDoubles[32];
char    ctrlStrings[8][64];

// Heartbeat blink
bool booflip = LOW;
unsigned long previousMillis = 0;
const long interval = 1000;


// ===============================
// DS API
// ===============================

void dsWriteBool(int index, bool value) {
    if (index < 0 || index >= 32) return;
    dsBools[index] = value;
}

void dsWriteDbl(int index, double value) {
    if (index < 0 || index >= 32) return;
    dsDoubles[index] = value;
}

void dsWriteString(int index, const char* value) {
    if (index < 0 || index >= 8 || value == nullptr) return;

    size_t len = strlen(value);
    if (len > 63) len = 63;

    memset(dsStrings[index], 0, 64);
    memcpy(dsStrings[index], value, len);
    dsStrings[index][63] = '\0';
}

bool dsReadBool(int index) {
    if (index < 0 || index >= 32) return false;
    return ctrlBools[index];
}

double dsReadDbl(int index) {
    if (index < 0 || index >= 32) return 0.0;
    return ctrlDoubles[index];
}
const char* dsReadString(int index) {
    if (index < 0 || index >= 8) return "";
    return ctrlStrings[index];
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

volatile uint32_t g_frameCounter       = 0;
volatile uint32_t g_rxFrameCounterGui  = 0;
volatile uint64_t g_timestampNetworkUs = 0;
volatile DsState  g_dsState            = DS_STATE_DISCONNECTED;

uint32_t g_lastInboundMs = 0;
uint32_t g_heartbeat = 0;


// ===============================
// UDP / NETWORK
// ===============================

WiFiUDP udp;

const uint8_t PACKET_COUNT_PER_FRAME = 4;

const size_t HEADER_SIZE = 4 + 4 + 8 + 1 + 1 + 1 + 1;

const size_t P0_PAYLOAD_SIZE = 4 + (16 * 8);
const size_t P1_PAYLOAD_SIZE = (16 * 8);
const size_t P2_PAYLOAD_SIZE = 4 * 64;
const size_t P3_PAYLOAD_SIZE = 4 * 64;

const size_t P0_PACKET_SIZE = HEADER_SIZE + P0_PAYLOAD_SIZE;
const size_t P1_PACKET_SIZE = HEADER_SIZE + P1_PAYLOAD_SIZE;
const size_t P2_PACKET_SIZE = HEADER_SIZE + P2_PAYLOAD_SIZE;
const size_t P3_PACKET_SIZE = HEADER_SIZE + P3_PAYLOAD_SIZE;

const size_t MAX_OUTBOUND_PACKET_SIZE = HEADER_SIZE + P2_PAYLOAD_SIZE;


// ===============================
// BINARY HELPERS
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

uint32_t packBoolBits() {
    uint32_t bits = 0;
    for (int i = 0; i < 32; i++) {
        if (dsBools[i]) bits |= (1UL << i);
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
    writeUint32(buffer, offset, g_frameCounter);
    writeUint32(buffer, offset, g_rxFrameCounterGui);

    g_timestampNetworkUs = (uint64_t)micros();
    writeUint64(buffer, offset, g_timestampNetworkUs);

    buffer[offset++] = (uint8_t)g_dsState;
    buffer[offset++] = packetIndex;
    buffer[offset++] = PACKET_COUNT_PER_FRAME;
    buffer[offset++] = 0;
}

void buildAndSendPacket0() {
    uint8_t buffer[P0_PACKET_SIZE];
    int offset = 0;

    writeCommonHeader(buffer, offset, 0);

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
// Option 3 patches applied:
//   ✔ Patch A: Ignore inbound packets until WiFi + GUI stable
//   ✔ Patch B: Guard against malformed/oversized packets

void handleInboundPacket(const uint8_t* buffer, size_t length) {

    // PATCH B — malformed packet guard
    if (length < HEADER_SIZE) return;
    if (length > MAX_OUTBOUND_PACKET_SIZE) return;

    int offset = 0;

    uint32_t frameIdFromGui = readUint32(buffer, offset);
    uint32_t ackFrameId     = readUint32(buffer, offset);
    uint64_t guiTimestampUs = readUint64(buffer, offset);

    uint8_t dsStateFromGui  = buffer[offset++];
    uint8_t packetIndex     = buffer[offset++];
    uint8_t packetCount     = buffer[offset++];
    offset++;

    // PATCH B — invalid index/count guard
    if (packetCount != PACKET_COUNT_PER_FRAME) return;
    if (packetIndex >= PACKET_COUNT_PER_FRAME) return;

    // PATCH A — ignore inbound until connected
    //if (g_dsState != DS_STATE_CONNECTED) return;

    g_rxFrameCounterGui = frameIdFromGui;
    g_lastInboundMs = millis();

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
    if (WiFi.status() == WL_CONNECTED) {
        digitalWrite(LED_BLUE, HIGH);
    } else {
        digitalWrite(LED_BLUE, LOW);
    }

    uint32_t now = millis();
    if (now - g_lastInboundMs < GUI_TIMEOUT_MS) {
        digitalWrite(LED_GREEN, LOW);
    } else {
        digitalWrite(LED_GREEN, HIGH);
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
    pinMode(LED_BLUE, OUTPUT);
    pinMode(LED_GREEN, OUTPUT);

    digitalWrite(LED_BLUE, LOW);
    digitalWrite(LED_GREEN, HIGH);

    ctrlServo.attach(SERVO_PIN);

    for (int i = 0; i < 32; i++) {
        dsBools[i] = false;
        dsDoubles[i] = 0.0;
    }
    for (int i = 0; i < 8; i++) {
        memset(dsStrings[i], 0, 64);
        strncpy(dsStrings[i], "INIT", 63);
        dsStrings[i][63] = '\0';
    }

    memset(ctrlBools, 0, sizeof(ctrlBools));
    memset(ctrlDoubles, 0, sizeof(ctrlDoubles));
    memset(ctrlStrings, 0, sizeof(ctrlStrings));

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

    // RECEIVE GUI → GIGA
    startTime = millis(); 

    int packetSize = udp.parsePacket();
    if (packetSize > 0) {
        uint8_t inboundBuffer[512];
        int len = udp.read(inboundBuffer, sizeof(inboundBuffer));
        if (len > 0) {
            handleInboundPacket(inboundBuffer, (size_t)len);
        }
    }

    updateDsState();
    updateLeds();

    // Example telemetry: analog inputs
    for (int i = 0; i < 8; i++) {
        int raw = analogRead(i);
        dsWriteDbl(i, (double)raw);
    }

    // Heartbeat in dsDoubles[31]
    g_heartbeat++;
    dsWriteDbl(31, (double)g_heartbeat);

    double joyval = dsReadDbl(4);
    int angle = ((joyval + 1.0) * 90.0);
    ctrlServo.write(angle);


    dsWriteDbl(13,angle);
    dsWriteDbl(15,joyval);
    dsWriteDbl(14,(udp.parsePacket()));
    // SEND OUTBOUND FRAME
    static uint32_t lastSendMs = 0;
    uint32_t nowMs = millis();

    dsWriteDbl(10, dsReadDbl(4));   // raw inbound
    dsWriteDbl(11, dsReadDbl(4) * 1000.0);
    dsWriteDbl(12, (int)(dsReadDbl(4) * 1000.0));
    dsWriteDbl(30, g_dsState);
    dsWriteDbl(29, millis() - g_lastInboundMs);



    if (nowMs - lastSendMs >= OUTBOUND_PERIOD_MS) {
        lastSendMs = nowMs;
        sendOutboundFrame();
    }

    // Heartbeat on dsBools[6]
    unsigned long currentMillis = millis();
    if (currentMillis - previousMillis >= interval) {
        previousMillis = currentMillis;
        booflip = !booflip;
        dsWriteBool(6, booflip); 
    }
    unsigned long endTime = millis(); // Capture the end time
    dsWriteDbl(16, endTime - startTime); // Calculate the elapsed time
}