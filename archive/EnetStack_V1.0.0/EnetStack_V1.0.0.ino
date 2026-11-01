#include <SPI.h>
#include <Ethernet.h>
#include <EthernetUdp.h>
#include <Wire.h>
#include "RTClib.h"

// ============================================================
// Network Configuration
// ============================================================
byte mac[] = { 0xDE, 0xAD, 0xBE, 0xEF, 0xFE, 0xED };

IPAddress ip(192, 168, 0, 51);
IPAddress dns(192, 168, 0, 1);
IPAddress gateway(192, 168, 0, 1);
IPAddress subnet(255, 255, 255, 0);

EthernetUDP Udp;
unsigned int localPort = 5005;

// Destination (Node server)
IPAddress nodeIP(192, 168, 0, 175);
unsigned int nodePort = 5006;

// ============================================================
// RTC
// ============================================================
RTC_DS3231 rtc;

// ============================================================
// Telemetry Arrays (Library Internals)
// ============================================================
bool     telemBoolArr[32];
int32_t  telemIntArr[32];
char     telemStrArr[8][64];

bool     healthBoolArr[8];
int32_t  healthIntArr[8];
char     healthStrArr[2][64];

// ============================================================
// Binary Telemetry Frame (Static Size)
// ============================================================
struct TelemetryFrame {
  uint32_t ts_sec;
  uint32_t ts_usec;

  uint8_t  telemBool[32];
  int32_t  telemInt[32];
  char     telemStr[8][64];

  uint8_t  healthBool[8];
  int32_t  healthInt[8];
  char     healthStr[2][64];
};

TelemetryFrame frame;

// ============================================================
// Public API (Library Functions)
// ============================================================
void telemBool(uint8_t index, bool value) {
  if (index < 32) telemBoolArr[index] = value;
}

void telemInt(uint8_t index, int32_t value) {
  if (index < 32) telemIntArr[index] = value;
}

void telemStr(uint8_t index, const char* value) {
  if (index < 8) strncpy(telemStrArr[index], value, 63);
}

void healthBool(uint8_t index, bool value) {
  if (index < 8) healthBoolArr[index] = value;
}

void healthInt(uint8_t index, int32_t value) {
  if (index < 8) healthIntArr[index] = value;
}

void healthStr(uint8_t index, const char* value) {
  if (index < 2) strncpy(healthStrArr[index], value, 63);
}

// ============================================================
// Heartbeat
// ============================================================
unsigned long lastHeartbeat = 0;

// ============================================================
// Setup
// ============================================================
void setup() {
  Serial.begin(115200);
  delay(1500);

  Wire.begin();
  rtc.begin();

  Ethernet.begin(mac, ip, dns, gateway, subnet);
  Udp.begin(localPort);

  memset(&frame, 0, sizeof(frame));
  memset(telemBoolArr, 0, sizeof(telemBoolArr));
  memset(telemIntArr, 0, sizeof(telemIntArr));
  memset(telemStrArr, 0, sizeof(telemStrArr));
  memset(healthBoolArr, 0, sizeof(healthBoolArr));
  memset(healthIntArr, 0, sizeof(healthIntArr));
  memset(healthStrArr, 0, sizeof(healthStrArr));

  Serial.println("Binary Telemetry + API Ready");
}

// ============================================================
// Loop
// ============================================================
void loop() {

  // ------------------------------------------------------------
  // 1. Heartbeat (1 second)
  // ------------------------------------------------------------
  if (millis() - lastHeartbeat >= 1000) {
    lastHeartbeat = millis();

    // Toggle heartbeat using API
    healthBool(0, !healthBoolArr[0]);

    // Human-readable RTC print
    DateTime now = rtc.now();
    char buf[32];
    snprintf(buf, sizeof(buf), "%04d-%02d-%02d %02d:%02d:%02d",
      now.year(), now.month(), now.day(),
      now.hour(), now.minute(), now.second()
    );
    Serial.print("RTC: ");
    Serial.println(buf);
  }

  // ------------------------------------------------------------
  // 2. Example sensor read
  // ------------------------------------------------------------
  telemInt(0, analogRead(A0));

  // ------------------------------------------------------------
  // 3. Timestamp for this frame
  // ------------------------------------------------------------
  DateTime now = rtc.now();
  frame.ts_sec  = now.unixtime();
  frame.ts_usec = micros() % 1000000;

  // ------------------------------------------------------------
  // 4. Copy API arrays into binary frame
  // ------------------------------------------------------------
  for (int i = 0; i < 32; i++) frame.telemBool[i] = telemBoolArr[i];
  for (int i = 0; i < 32; i++) frame.telemInt[i]  = telemIntArr[i];
  for (int i = 0; i < 8; i++)  strcpy(frame.telemStr[i], telemStrArr[i]);

  for (int i = 0; i < 8; i++)  frame.healthBool[i] = healthBoolArr[i];
  for (int i = 0; i < 8; i++)  frame.healthInt[i]  = healthIntArr[i];
  for (int i = 0; i < 2; i++)  strcpy(frame.healthStr[i], healthStrArr[i]);

  // ------------------------------------------------------------
  // 5. Send the entire binary frame in ONE UDP packet
  // ------------------------------------------------------------
  Udp.beginPacket(nodeIP, nodePort);
  Udp.write((uint8_t*)&frame, sizeof(frame));
  Udp.endPacket();

  delay(40); // ~25 Hz
}