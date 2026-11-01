// =========================================================
// BLOCK 1: Includes, Config Struct, Enums, Forward Declarations
// =========================================================

#include <Arduino.h>
#include <WiFi.h>
#include <WiFiUdp.h>
#include <Arduino_JSON.h>
#include "stm32h7xx_hal.h"

// ---------------------------------------------------------
// LED Pin (GIGA R1 WiFi onboard blue LED)
// ---------------------------------------------------------
//#define LED_BLUE 24   // active-LOW

// ---------------------------------------------------------
// Telemetry Configuration Struct
// ---------------------------------------------------------
struct TelemetryConfig {
    const char* ssid;
    const char* password;

    IPAddress localIP;
    IPAddress gateway;
    IPAddress subnet;

    IPAddress serverIP;
    uint16_t localPort;
    uint16_t remotePort;

    uint16_t refreshHz;

    bool enableLogging;
    enum LogFormat { LOG_CSV, LOG_JSON } logFormat;
};

// ---------------------------------------------------------
// Health Keys (USB removed)
// ---------------------------------------------------------
enum HealthKey {
    wifiConnected,
    serverReachable,
    timeSynced,

    wifiReconnectAttempts,
    controlAgeMs,
    latencyMicros,
    clockDriftMicros,
    cpuLoadPercent
};

// ---------------------------------------------------------
// Forward Declarations
// ---------------------------------------------------------
void maintainWiFi();
void sendTelemetryIfDue();
void receiveControlIfAvailable();
void updateNetworkTime(uint64_t serverTimeMicros);
uint64_t NetworkTimeMicros();
void updateHealth();

void TelemetryBool(uint8_t index, bool value);
void TelemetryDbl(uint8_t index, double value);
void TelemetryStr(uint8_t index, const String& value);

bool   ControlBool(uint8_t index);
double ControlDbl(uint8_t index);
String ControlStr(uint8_t index);

double ControlJoy(uint8_t axis);
bool   ControlJoyBtn(uint8_t btn);
// =========================================================
// BLOCK 2: Globals, Internal State (USB-free)
// =========================================================

// ---------------------------------------------------------
// Configuration
// ---------------------------------------------------------
TelemetryConfig config;

// ---------------------------------------------------------
// WiFi + UDP
// ---------------------------------------------------------
WiFiUDP udp;
bool wifiConnectedFlag = false;
unsigned long lastWifiAttempt = 0;
const unsigned long wifiRetryInterval = 2000; // ms

// ---------------------------------------------------------
// Telemetry Storage (User-facing)
// ---------------------------------------------------------
bool   t_bool[16];
double t_dbl[16];
String t_str[16];

// ---------------------------------------------------------
// Control Storage (Inbound from server.js)
// ---------------------------------------------------------
bool   c_bool[16];
double c_dbl[16];
String c_str[16];

// Joystick (Inbound from server.js)
double joyAxis[8];
bool   joyBtn[16];

// Timestamp of last control packet
unsigned long lastControlPacketMillis = 0;

// ---------------------------------------------------------
// Network Time Grounding
// ---------------------------------------------------------
uint64_t networkTimeMicros = 0;
uint64_t localMicrosAtSync = 0;
bool     networkTimeValid = false;

// ---------------------------------------------------------
// Health State Storage
// ---------------------------------------------------------
bool   health_bools[16];
double health_doubles[16];

// ---------------------------------------------------------
// Telemetry Timing
// ---------------------------------------------------------
unsigned long lastTelemetryMicros = 0;
uint32_t telemetryIntervalMicros = 20000; // default 50 Hz

// ---------------------------------------------------------
// RTC Handle (STM32H7 onboard RTC)
// ---------------------------------------------------------
RTC_HandleTypeDef hrtc;

// ---------------------------------------------------------
// CPU Load Measurement
// ---------------------------------------------------------
unsigned long lastLoopMicros = 0;
double cpuLoadEMA = 0.0;
// =========================================================
// BLOCK 3: setup() and loop()
// =========================================================

void setup() {
    delay(2000);

    // LED init
    pinMode(LED_BLUE, OUTPUT);
    digitalWrite(LED_BLUE, HIGH);   // active-LOW → OFF

    // -----------------------------------------------------
    // 1. User WiFi + Network Configuration
    // -----------------------------------------------------
    config.ssid     = "YOUR_WIFI_NAME";
    config.password = "REDACTED_WIFI_PASSWORD";

    config.localIP    = IPAddress(192,168,0,50);
    config.gateway    = IPAddress(192,168,0,1);
    config.subnet     = IPAddress(255,255,255,0);

    config.serverIP   = IPAddress(192,168,0,175);
    config.localPort  = 5006;
    config.remotePort = 5005;

    config.refreshHz = 50;


     // -----------------------------------------------------
    // 3. Telemetry timing
    // -----------------------------------------------------
    telemetryIntervalMicros = 1000000UL / config.refreshHz;
    lastTelemetryMicros     = micros();

    // -----------------------------------------------------
    // 4. WiFi + UDP
    // -----------------------------------------------------
    WiFi.config(config.localIP, config.gateway, config.subnet);
    WiFi.begin(config.ssid, config.password);

    udp.begin(config.localPort);

    // -----------------------------------------------------
    // 5. Health init
    // -----------------------------------------------------
    health_bools[wifiConnected]   = false;
    health_bools[serverReachable] = false;
    health_bools[timeSynced]      = false;

    lastControlPacketMillis = millis();
    lastLoopMicros = micros();
}

void loop() {
    unsigned long loopStart = micros();

    maintainWiFi();
    receiveControlIfAvailable();
    sendTelemetryIfDue();
    updateHealth();

    // -----------------------------------------------------
    // CPU Load
    // -----------------------------------------------------
    unsigned long loopEnd = micros();
    double loopTime = (double)(loopEnd - loopStart);

    cpuLoadEMA = (cpuLoadEMA * 0.95) + (loopTime * 0.05);
    health_doubles[cpuLoadPercent] =
        (cpuLoadEMA / telemetryIntervalMicros) * 100.0;

    // -----------------------------------------------------
    // TEMPORARY TEST: Analog Inputs A0–A6 → TelemetryDbl[0–6]
    // -----------------------------------------------------
    for (int i = 0; i < 7; i++) {
        int pin = A0 + i;
        int raw = analogRead(pin);
        TelemetryDbl(i, (double)raw);
    }

    // -----------------------------------------------------
    // TEMPORARY TEST: 1-second heartbeat → TelemetryBool[1]
    // -----------------------------------------------------
    static unsigned long lastBeat = 0;
    unsigned long nowMs = millis();

    if (nowMs - lastBeat >= 1000) {
        lastBeat = nowMs;
        t_bool[1] = !t_bool[1];
    }
}
// =========================================================
// BLOCK 4: WiFi Subsystem (Non-blocking Auto-Reconnect)
// =========================================================

void maintainWiFi() {
    uint8_t status = WiFi.status();

    if (status == WL_CONNECTED) {
        wifiConnectedFlag = true;
        health_bools[wifiConnected] = true;

        // BLUE LED OFF when connected (active-LOW)
        digitalWrite(LED_BLUE, HIGH);
        return;
    }

    // Not connected
    wifiConnectedFlag = false;
    health_bools[wifiConnected] = false;

    // BLUE LED ON while connecting (active-LOW)
    digitalWrite(LED_BLUE, LOW);

    unsigned long now = millis();
    if (now - lastWifiAttempt >= wifiRetryInterval) {
        lastWifiAttempt = now;
        health_doubles[wifiReconnectAttempts] += 1.0;

        WiFi.disconnect();
        WiFi.begin(config.ssid, config.password);
    }
}
// =========================================================
// BLOCK 5: UDP Telemetry + Control Subsystem (timestamp_ms)
// =========================================================

// ---------------------------------------------------------
// receiveControlIfAvailable()
// Non-blocking UDP receive + JSON parse.
// Updates control arrays, joystick, and network time.
// ---------------------------------------------------------
void receiveControlIfAvailable() {
    int packetSize = udp.parsePacket();
    if (packetSize <= 0) return;

    String jsonStr;
    jsonStr.reserve(packetSize + 4);

    while (udp.available()) {
        jsonStr += (char)udp.read();
    }

    JSONVar root = JSON.parse(jsonStr);
    if (JSON.typeof(root) != "object") return;

    // -----------------------------------------------------
    // Extract timestamp_ms (simple, numeric, 64-bit safe)
    // -----------------------------------------------------
    if (root.hasOwnProperty("timestamp_ms")) {
        uint64_t serverTime = (uint64_t)(double)root["timestamp_ms"];
        updateNetworkTime(serverTime);
        health_bools[timeSynced] = true;
    }

    // -----------------------------------------------------
    // Extract control bools
    // -----------------------------------------------------
    if (root.hasOwnProperty("c_bools")) {
        JSONVar arr = root["c_bools"];
        for (int i = 0; i < 16; i++) {
            if (i < arr.length()) c_bool[i] = (bool)arr[i];
        }
    }

    // -----------------------------------------------------
    // Extract control doubles
    // -----------------------------------------------------
    if (root.hasOwnProperty("c_doubles")) {
        JSONVar arr = root["c_doubles"];
        for (int i = 0; i < 16; i++) {
            if (i < arr.length()) c_dbl[i] = (double)arr[i];
        }
    }

    // -----------------------------------------------------
    // Extract control strings
    // -----------------------------------------------------
    if (root.hasOwnProperty("c_strings")) {
        JSONVar arr = root["c_strings"];
        for (int i = 0; i < 16; i++) {
            if (i < arr.length()) c_str[i] = (const char*)arr[i];
        }
    }

    // -----------------------------------------------------
    // Extract joystick axes
    // -----------------------------------------------------
    if (root.hasOwnProperty("c_joy")) {
        JSONVar arr = root["c_joy"];
        for (int i = 0; i < 8; i++) {
            if (i < arr.length()) joyAxis[i] = (double)arr[i];
        }
    }

    // -----------------------------------------------------
    // Extract joystick buttons
    // -----------------------------------------------------
    if (root.hasOwnProperty("c_joyBtn")) {
        JSONVar arr = root["c_joyBtn"];
        for (int i = 0; i < 16; i++) {
            if (i < arr.length()) joyBtn[i] = (bool)arr[i];
        }
    }

    // -----------------------------------------------------
    // Mark control packet as fresh
    // -----------------------------------------------------
    lastControlPacketMillis = millis();
    health_bools[serverReachable] = true;
}


// ---------------------------------------------------------
// sendTelemetryIfDue()
// Sends telemetry + health JSON at refreshHz.
// ---------------------------------------------------------
void sendTelemetryIfDue() {
    unsigned long nowMicros = micros();
    if (nowMicros - lastTelemetryMicros < telemetryIntervalMicros) return;
    lastTelemetryMicros = nowMicros;

    if (!wifiConnectedFlag) return;

    JSONVar root;

    // -----------------------------------------------------
    // Timestamp in milliseconds
    // -----------------------------------------------------
    root["timestamp_ms"] = (double)(NetworkTimeMicros() / 1000ULL);

    // -----------------------------------------------------
    // Telemetry arrays
    // -----------------------------------------------------
    JSONVar tb; for (int i = 0; i < 16; i++) tb[i] = t_bool[i];
    JSONVar td; for (int i = 0; i < 16; i++) td[i] = t_dbl[i];
    JSONVar ts; for (int i = 0; i < 16; i++) ts[i] = t_str[i];

    root["telemetry"]["t_bools"]   = tb;
    root["telemetry"]["t_doubles"] = td;
    root["telemetry"]["t_strings"] = ts;

    // -----------------------------------------------------
    // Health arrays
    // -----------------------------------------------------
    JSONVar hb;
    JSONVar hd;

    hb["wifiConnected"]   = health_bools[wifiConnected];
    hb["serverReachable"] = health_bools[serverReachable];
    hb["timeSynced"]      = health_bools[timeSynced];

    hd["wifiReconnectAttempts"] = health_doubles[wifiReconnectAttempts];
    hd["controlAgeMs"]          = health_doubles[controlAgeMs];
    hd["latencyMicros"]         = health_doubles[latencyMicros];
    hd["clockDriftMicros"]      = health_doubles[clockDriftMicros];
    hd["cpuLoadPercent"]        = health_doubles[cpuLoadPercent];

    root["health"]["h_bools"]   = hb;
    root["health"]["h_doubles"] = hd;

    // -----------------------------------------------------
    // Serialize JSON
    // -----------------------------------------------------
    String out = JSON.stringify(root);

    // -----------------------------------------------------
    // Send UDP packet
    // -----------------------------------------------------
    udp.beginPacket(config.serverIP, config.remotePort);
    udp.write(out.c_str());
    udp.endPacket();
}
// =========================================================
// BLOCK 6: Network-Time Grounding Subsystem
// =========================================================

void updateNetworkTime(uint64_t serverTimeMicros) {
    networkTimeMicros = serverTimeMicros;
    localMicrosAtSync = micros();
    networkTimeValid  = true;

    uint64_t now = micros();
    uint64_t rtt = now - localMicrosAtSync;
    health_doubles[latencyMicros] = (double)(rtt / 2);

    int64_t drift = (int64_t)serverTimeMicros - (int64_t)now;
    health_doubles[clockDriftMicros] = (double)drift;
}

uint64_t NetworkTimeMicros() {
    if (!networkTimeValid) return (uint64_t)micros();
    uint64_t elapsed = (uint64_t)(micros() - localMicrosAtSync);
    return networkTimeMicros + elapsed;
}


// =========================================================
// BLOCK 7: Health Subsystem (USB removed)
// =========================================================

bool Health(HealthKey key) {
    return health_bools[key];
}

double HealthDbl(HealthKey key) {
    return health_doubles[key];
}

void updateHealth() {
    unsigned long now = millis();

    health_bools[wifiConnected] = wifiConnectedFlag;

    unsigned long age = now - lastControlPacketMillis;
    health_doubles[controlAgeMs] = (double)age;

    if (age > (2000 / config.refreshHz))
        health_bools[serverReachable] = false;
}


// =========================================================
// BLOCK 8: (Unused — USB Logging Removed)
// =========================================================


// =========================================================
// BLOCK 9: (Unused — USB Logging Removed)
// =========================================================


// =========================================================
// BLOCK 10: Utility Functions + Accessors (USB-free)
// =========================================================

String buildLogFilenameFromRTC() {
    RTC_DateTypeDef date;
    RTC_TimeTypeDef time;

    HAL_RTC_GetDate(&hrtc, &date, RTC_FORMAT_BIN);
    HAL_RTC_GetTime(&hrtc, &time, RTC_FORMAT_BIN);

    char buf[32];
    snprintf(buf, sizeof(buf),
             "%04u%02u%02u_%02u%02u.csv",
             2000 + date.Year,
             date.Month,
             date.Date,
             time.Hours,
             time.Minutes);

    return String(buf);
}

bool ControlBool(uint8_t index) {
    if (index >= 16) return false;
    return c_bool[index];
}

double ControlDbl(uint8_t index) {
    if (index >= 16) return 0.0;
    return c_dbl[index];
}

String ControlStr(uint8_t index) {
    if (index >= 16) return "";
    return c_str[index];
}

double ControlJoy(uint8_t axis) {
    if (axis >= 8) return 0.0;
    return joyAxis[axis];
}

bool ControlJoyBtn(uint8_t btn) {
    if (btn >= 16) return false;
    return joyBtn[btn];
}

void TelemetryBool(uint8_t index, bool value) {
    if (index < 16) t_bool[index] = value;
}

void TelemetryDbl(uint8_t index, double value) {
    if (index < 16) t_dbl[index] = value;
}

void TelemetryStr(uint8_t index, const String& value) {
    if (index < 16) t_str[index] = value;
}