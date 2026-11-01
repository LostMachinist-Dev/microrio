#include <Wire.h>
#include <RTClib.h>
#include <RemoteIO.h>
#include <Ethernet.h>

byte mac[] = { 0xA8, 0x61, 0x0A, 0xAE, 0x35, 0x58 };

RemoteIO rio;
RTC_DS3231 rtc;

const int ledPin = LED_BUILTIN;
int ledState = LOW;

unsigned long previousMillis = 0;
const unsigned long interval = 1000;

const int sensorPin = A0;
int sensorValue = 0;

// RTC time sources for RemoteIO
uint32_t RTCs() {
    DateTime now = rtc.now();
    return now.unixtime();   // seconds since 1970
}

uint32_t RTCus() {
    return micros();
}

void setup() {
    Serial.begin(115200);
    delay(2000);

    Serial.println("=== SETUP BEGIN ===");

    // ------------------------------------------------------------
    // 1. Initialize I2C bus FIRST
    // ------------------------------------------------------------
    Serial.println("Initializing I2C...");
    Wire.begin();

    // ------------------------------------------------------------
    // 2. Initialize RTC BEFORE calling lostPower(), now(), adjust()
    // ------------------------------------------------------------
    Serial.println("Initializing RTC...");
    bool rtc_ok = false;

    if (!rtc.begin()) {
        Serial.println("RTC not found! Continuing without RTC.");
    } else {
        Serial.println("RTC detected.");
        rtc_ok = true;

        Serial.println("Checking RTC power state...");
        if (rtc.lostPower()) {
            Serial.println("RTC lost power, setting compile time.");
            rtc.adjust(DateTime(F(__DATE__), F(__TIME__)));
        }
    }

    // ------------------------------------------------------------
    // 3. Network configuration
    // ------------------------------------------------------------
    IPAddress controllerIp(192,168,0,50);   // Arduino IP
    IPAddress consoleIp(192,168,0,8);       // PC/server IP
    uint16_t controllerPort = 5005;
    uint16_t consolePort    = 5006;

    Serial.println("Starting Ethernet...");
    Ethernet.begin(mac, controllerIp);

    Serial.print("Local IP: ");
    Serial.println(Ethernet.localIP());

    Serial.print("Link status: ");
    Serial.println(Ethernet.linkStatus());   // 1 = LinkON

    // ------------------------------------------------------------
    // 4. Initialize RemoteIO
    // ------------------------------------------------------------
    Serial.println("Starting RemoteIO...");
    rio.begin(controllerIp, consoleIp, controllerPort, consolePort);

    // Optional time sources (safe now that RTC is initialized)
    rio.setTimeSourceSeconds(RTCs);
    rio.setTimeSourceMicros(RTCus);

    rio.setRate(25);   // 25 Hz telemetry

    // ------------------------------------------------------------
    // 5. Local hardware setup
    // ------------------------------------------------------------
    pinMode(ledPin, OUTPUT);

    Serial.println("=== SETUP COMPLETE ===");
}

void loop() {

    // ============================================================
    // LED Blink
    // ============================================================
    unsigned long currentMillis = millis();
    if (currentMillis - previousMillis >= interval) {
        previousMillis = currentMillis;
        ledState = !ledState;
        digitalWrite(ledPin, ledState);
    }

    // ============================================================
    // Sensor Read
    // ============================================================
    sensorValue = analogRead(sensorPin);
    Serial.println(sensorValue);

    // ============================================================
    // Telemetry (write BEFORE poll)
    // ============================================================

    // Mirror control → telemetry
    rio.telemInt(4, rio.controlInt(4));
    rio.telemStr(1, rio.controlStr(1));

    // Basic telemetry
    rio.telemBool(0, true);
    rio.telemBool(1, ledState);
    rio.telemInt(0, sensorValue);
    rio.telemInt(9, sensorValue);

    // Floats (replacing doubles)
    rio.telemFloat(0, 42.1f);
    rio.telemFloat(1, rio.controlFloat(1));

    // ============================================================
    // STATUS PACKET POPULATION
    // ============================================================
    TelemetryStatus &S = rio.status();

    S.connection_ok = 1;   // Always true unless you add logic
    S.rtc_ok        = rtc.begin();   // Simple check
    S.ethernet_ok   = (Ethernet.linkStatus() == LinkON);
    S.wifi_ok       = 0;   // No WiFi on this board
    S.sd_logging_ok = 1;   // Placeholder
    S.node_id       = 1;   // Example
    S.cpu_temp_c    = 0;   // Placeholder
    S.battery_mv    = 0;   // Placeholder

    // Firmware version
    strncpy(S.fw_version_str, "0.0.1", 16);

    // ============================================================
    // CONTROL STATUS HANDLING (time sync, etc.)
    // ============================================================
    ControlStatus CS = rio.getControlStatus();

    if (CS.syncTime && rtc.begin()) {
        rtc.adjust(DateTime(CS.unix_sec));
        Serial.println("RTC time synced from console.");
    }

    // ============================================================
    // NOW poll (this sends telemetry + status)
    // ============================================================
    rio.poll();
}