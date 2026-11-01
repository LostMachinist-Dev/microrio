#include <SPI.h>
#include <Ethernet.h>
#include <Wire.h>
#include <RTClib.h>

// -----------------------------
// User‑tunable loop rate
// -----------------------------
float update_rate = 10.0;  // Hz
unsigned long loop_period_us = 100000;

// -----------------------------
// Network
// -----------------------------
byte mac[] = { 0xDE, 0xAD, 0xBE, 0xEF, 0xFE, 0xED };
IPAddress ip(192,168,0,50);
EthernetServer server(80);

// -----------------------------
// RTC
// -----------------------------
RTC_DS3231 rtc;
unsigned long lastRTC = 0;
uint32_t cached_unixtime = 0;
float cached_tempC = 0;
char cached_time_str[12];

// -----------------------------
// Loop metric
// -----------------------------
float duty_pct = 0;

// -----------------------------
// LED control
// -----------------------------
const int LED_PIN = 13;

// -----------------------------
// Auto‑Reconnect Helper
// -----------------------------
void checkEthernet() {
  Ethernet.maintain();
  if (Ethernet.linkStatus() == LinkOFF) {
    Ethernet.begin(mac, ip);
    server.begin();
  }
}

// -----------------------------
// HTTP Handlers
// -----------------------------
void sendIndex(EthernetClient &client) {
  client.println("HTTP/1.1 200 OK");
  client.println("Content-Type: text/html");
  client.println("Connection: close");
  client.println();

  client.println("<!DOCTYPE html><html><body style='margin:0;padding:0;font-family:sans-serif;'>");

  // --- TOP BAR ---
  client.println("<div style='display:flex;justify-content:space-between;"
                 "align-items:center;padding:8px 12px;background:#eee;"
                 "border-bottom:1px solid #ccc;'>");

  // LEFT: JSON telemetry (single line)
  client.println("<div id='jsonbox' "
                 "style='font-family:monospace;font-size:12px;white-space:nowrap;"
                 "overflow:hidden;text-overflow:ellipsis;max-width:60%;'>"
                 "loading..."
                 "</div>");

  // RIGHT: Load Custom UI button
  client.println("<div>");
  client.println("<button onclick=\"pickFile()\" "
                 "style='padding:6px 12px;font-size:14px;'>Load Custom UI</button>");
  client.println("<input type='file' id='fileInput' accept='text/html' style='display:none'>");
  client.println("</div>");

  client.println("</div>"); // end top bar

  // --- MAIN IFRAME ---
  client.println("<iframe id='customFrame' "
                 "style='width:100%;height:88vh;border:none;'></iframe>");

  // --- SCRIPT ---
  client.println("<script>");

  // JSON updater (upper-left)
  client.println("async function updateJSON(){");
  client.println("  try {");
  client.println("    let r = await fetch('/telemetry');");
  client.println("    let j = await r.json();");
  client.println("    document.getElementById('jsonbox').textContent = JSON.stringify(j);");
  client.println("  } catch(e) {");
  client.println("    document.getElementById('jsonbox').textContent = 'no data';");
  client.println("  }");
  client.println("  setTimeout(updateJSON, 1000/" + String(update_rate) + ");");
  client.println("}");
  client.println("updateJSON();");

  // File picker + iframe loader
  client.println("function pickFile(){ document.getElementById('fileInput').click(); }");

  client.println("document.getElementById('fileInput').addEventListener('change', function(){");
  client.println("  const file = this.files[0];");
  client.println("  if(!file) return;");
  client.println("  const reader = new FileReader();");
  client.println("  reader.onload = function(e){");
  client.println("    document.getElementById('customFrame').srcdoc = e.target.result;");
  client.println("  };");
  client.println("  reader.readAsText(file);");
  client.println("});");

  client.println("</script>");
  client.println("</body></html>");
}

void sendTelemetry(EthernetClient &client,
                   int a0,
                   float tempC,
                   uint32_t unixtime,
                   const char* time_str,
                   float duty_pct)
{
  client.println("HTTP/1.1 200 OK");
  client.println("Content-Type: application/json");
  client.println("Connection: close");
  client.println();

  client.print("{");
  client.print("\"a0\":"); client.print(a0); client.print(",");
  client.print("\"tempC\":"); client.print(tempC, 2); client.print(",");
  client.print("\"unixtime\":"); client.print(unixtime); client.print(",");
  client.print("\"time_str\":\""); client.print(time_str); client.print("\",");

  client.print("\"duty_pct\":"); client.print(duty_pct, 2);
  client.print("}");
}

// -----------------------------
// Setup
// -----------------------------
void setup() {
  Wire.begin();
  rtc.begin();

  if (rtc.lostPower()) {
    rtc.adjust(DateTime(F(__DATE__), F(__TIME__)));
  }

  pinMode(LED_PIN, OUTPUT);
  digitalWrite(LED_PIN, LOW);

  Ethernet.begin(mac, ip);
  server.begin();

  loop_period_us = (unsigned long)(1000000.0 / update_rate);
}

// -----------------------------
// Main Loop
// -----------------------------
void loop() {
  unsigned long loop_start = micros();

  // 1. Update RTC (1 Hz)
  if (millis() - lastRTC >= 1000) {
    lastRTC = millis();
    DateTime now = rtc.now();

    cached_unixtime = now.unixtime();
    cached_tempC = rtc.getTemperature();

    snprintf(cached_time_str, sizeof(cached_time_str),
             "%02d:%02d:%02d",
             now.hour(), now.minute(), now.second());
  }

  // 2. Sensor reads
  int a0 = analogRead(A0);

  // 3. HTTP handling
  EthernetClient client = server.available();
  if (client) {
    String req = client.readStringUntil('\r');
    client.flush();

    if (req.startsWith("GET / ")) {
      sendIndex(client);
    }
    else if (req.startsWith("GET /telemetry")) {
      sendTelemetry(client,
                    a0,
                    cached_tempC,
                    cached_unixtime,
                    cached_time_str,
                    duty_pct);
    }
    else if (req.startsWith("GET /cmd")) {
      int ledPos = req.indexOf("led=");
      if (ledPos > 0) {
        char val = req.charAt(ledPos + 4);
        if (val == '1') digitalWrite(LED_PIN, HIGH);
        if (val == '0') digitalWrite(LED_PIN, LOW);
      }

      client.println("HTTP/1.1 200 OK");
      client.println("Content-Type: text/plain");
      client.println("Connection: close");
      client.println();
      client.println("OK");
    }

    delay(1);
    client.stop();
  }

  // 4. Duty cycle
  unsigned long busy = micros() - loop_start;

  if (busy >= loop_period_us) {
    duty_pct = 100.0;
  } else {
    duty_pct = (busy * 100.0) / loop_period_us;
    delayMicroseconds(loop_period_us - busy);
  }

  // 5. Auto‑Reconnect
  checkEthernet();
}