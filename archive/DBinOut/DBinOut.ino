#include <Arduino.h>
#include <WiFi.h>
#include <WiFiUdp.h>
#include <Arduino_JSON.h>

// ===============================
// USER DATA ARRAYS
// ===============================
bool    dsBools[32];
double  dsDoubles[16];
float   dsFloats[16];
int32_t dsInts[16];
String  dsStrings[8];


// ===============================
// WRITE HELPERS
// ===============================
enum DataType { DS_BOOL, DS_DOUBLE, DS_FLOAT, DS_INT, DS_STRING };

void dsWrite(int index, double value, DataType type) {
    switch (type) {
        case DS_BOOL:   if (index < 32) dsBools[index]   = (bool)value; break;
        case DS_DOUBLE: if (index < 16) dsDoubles[index] = value;       break;
        case DS_FLOAT:  if (index < 16) dsFloats[index]  = (float)value; break;
        case DS_INT:    if (index < 16) dsInts[index]    = (int32_t)value; break;
        default: break;
    }
}

void dsWrite(int index, const String &value, DataType type) {
    if (type == DS_STRING && index < 8)
        dsStrings[index] = value;
}

// ===============================
// READ HELPERS
// ===============================
double dsRead(int index, DataType type) {
    switch (type) {
        case DS_BOOL:   return (index < 32) ? dsBools[index]   : 0;
        case DS_DOUBLE: return (index < 16) ? dsDoubles[index] : 0;
        case DS_FLOAT:  return (index < 16) ? dsFloats[index]  : 0;
        case DS_INT:    return (index < 16) ? dsInts[index]    : 0;
        default:        return 0;
    }
}

String dsReadString(int index) {
    return (index < 8) ? dsStrings[index] : "";
}

// ===============================
// NETWORK CONFIG
// ===============================
const char* ssid     = "YOUR_WIFI_NAME";
const char* password = "REDACTED_WIFI_PASSWORD";

WiFiUDP udp;
const int LOCAL_PORT  = 5005;
const int REMOTE_PORT = 5006;

IPAddress guiIP(192,168,0,175);

unsigned long lastSend = 0;

// ===============================
// SERIALIZATION
// ===============================
String serializeOutbound() {
    JSONVar doc;

    JSONVar jb;
    for (int i = 0; i < 32; i++) jb[i] = dsBools[i];
    doc["bools"] = jb;

    JSONVar jd;
    for (int i = 0; i < 16; i++) jd[i] = dsDoubles[i];
    doc["doubles"] = jd;

    JSONVar jf;
    for (int i = 0; i < 16; i++) jf[i] = dsFloats[i];
    doc["floats"] = jf;

    JSONVar ji;
    for (int i = 0; i < 16; i++) ji[i] = dsInts[i];
    doc["ints"] = ji;

    JSONVar js;
    for (int i = 0; i < 8; i++) js[i] = dsStrings[i];
    doc["strings"] = js;

    return JSON.stringify(doc);
}


bool deserializeInbound(const String &json) {
    JSONVar doc = JSON.parse(json);
    if (JSON.typeof(doc) == "undefined") return false;

    for (int i = 0; i < 32; i++)
        dsBools[i] = (bool)doc["bools"][i];

    for (int i = 0; i < 16; i++)
        dsDoubles[i] = (double)doc["doubles"][i];

    for (int i = 0; i < 16; i++)
        dsFloats[i] = (double)doc["floats"][i];

    for (int i = 0; i < 16; i++)
        dsInts[i] = (int)doc["ints"][i];

    for (int i = 0; i < 8; i++)
        dsStrings[i] = (const char*)doc["strings"][i];

    return true;
}


// ===============================
// SETUP
// ===============================
void setup() {

    pinMode(LED_BLUE, OUTPUT);
    digitalWrite(LED_BLUE, LOW);   // LED ON while connecting

    // Hard reset the WiFi module
    WiFi.end();
    delay(200);

    // ---- STATIC IP CONFIG ----
    IPAddress ip(192,168,0,50);
    IPAddress gateway(192,168,0,1);
    IPAddress subnet(255,255,255,0);
    WiFi.config(ip, gateway, subnet);
    // ---------------------------

    WiFi.begin(ssid, password);
    while (WiFi.status() != WL_CONNECTED) {
        delay(100);
    }

    digitalWrite(LED_BLUE, HIGH);    // LED OFF once connected

    udp.begin(LOCAL_PORT);

    for (int i = 0; i < 8; i++)
        dsStrings[i] = "INIT";
}

// ===============================
// LOOP
// ===============================
void loop() {

    // ----------------------------
    // RECEIVE GUI → Arduino
    // ----------------------------
    int packetSize = udp.parsePacket();
    if (packetSize > 0) {

    char buffer[8192];
    int len = udp.read(buffer, sizeof(buffer) - 1);
    if (len > 0) buffer[len] = '\0';

    deserializeInbound(String(buffer));
  }

    // ----------------------------
    // UPDATE OUTBOUND DATA
    // ----------------------------
    for (int i = 0; i < 8; i++) {
        int raw = analogRead(i);
        dsWrite(i, (float)raw, DS_FLOAT);
    }


    // ----------------------------
    // SEND OUTBOUND (20 Hz)
    // ----------------------------
    if (millis() - lastSend >= 1000) {
        lastSend = millis();
        String json = serializeOutbound();
        static int32_t pkt = 0;
        dsWrite(0, pkt++, DS_INT);
        udp.beginPacket(guiIP, REMOTE_PORT);
        udp.print(json);
        udp.endPacket();
    }
}