/*
  GIGA R1 WiFi - 6‑channel Web Oscilloscope + External RGB LED Controls
  Dark theme + Bahnschrift/D-DIN font
  - Serves HTML/JS at "/"
  - Streams JSON data at "/data"
  - Toggles external RGB LED at "/led?c=R|G|B"
*/

#include <SPI.h>
#include <WiFi.h>

char ssid[] = "YOUR_WIFI_NAME";        
char pass[] = "REDACTED_WIFI_PASSWORD";      
int status = WL_IDLE_STATUS;

WiFiServer server(80);

// External RGB LED pins (active‑LOW)
int RState = HIGH;
int GState = HIGH;
int BState = LOW;

// ---------------------------------------------------------
//  Print WiFi status
// ---------------------------------------------------------
void printWifiStatus() {
  Serial.print("SSID: ");
  Serial.println(WiFi.SSID());

  IPAddress ip = WiFi.localIP();
  Serial.print("IP Address: ");
  Serial.println(ip);

  long rssi = WiFi.RSSI();
  Serial.print("signal strength (RSSI):");
  Serial.print(rssi);
  Serial.println(" dBm");
}

// ---------------------------------------------------------
//  Setup
// ---------------------------------------------------------
void setup() {
  Serial.begin(9600);
  while (!Serial) {}

  // External RGB LED setup (active‑LOW)
pinMode(LED_RED, OUTPUT);
pinMode(LED_GREEN, OUTPUT);
pinMode(LED_BLUE, OUTPUT);

digitalWrite(LED_RED, RState);   // OFF
digitalWrite(LED_GREEN, GState); // OFF
digitalWrite(LED_BLUE, BState);  // OFF


  if (WiFi.status() == WL_NO_MODULE) {
    Serial.println("Communication with WiFi module failed!");
    while (true);
  }

  while (status != WL_CONNECTED) {
    Serial.print("Attempting to connect to SSID: ");
    Serial.println(ssid);
    status = WiFi.begin(ssid, pass);
    delay(10000);
  }

  server.begin();
  printWifiStatus();
}

// ---------------------------------------------------------
//  Main loop
// ---------------------------------------------------------
void loop() {
  WiFiClient client = server.available();
  if (!client) return;

  Serial.println("new client");

  String req = client.readStringUntil('\r');
  Serial.println(req);
  client.readStringUntil('\n');

  // ---------------------------------------------------------
  //  /led?c=R|G|B  → toggle external RGB LED
  // ---------------------------------------------------------
  if (req.startsWith("GET /led")) {

    char channel = 'X';
    int idx = req.indexOf("c=");
    if (idx != -1 && idx + 2 < req.length()) {
      channel = req.charAt(idx + 2);
    }

if (channel == 'R') {
  RState = !RState;
  if (RState==LOW) {digitalWrite(LED_RED, LOW);}
  else {digitalWrite(LED_RED, HIGH);
  }
}

if (channel == 'G') {
  GState = !GState;
  if (GState==LOW) {digitalWrite(LED_GREEN, LOW);}
  else {digitalWrite(LED_GREEN, HIGH);
  }
}

if (channel == 'B') {
  BState = !BState;
  if (BState==LOW) {digitalWrite(LED_BLUE, LOW);}
  else {digitalWrite(LED_BLUE, HIGH);
  }
}

    client.println("HTTP/1.1 200 OK");
    client.println("Content-Type: text/plain");
    client.println("Connection: close");
    client.println();
    client.println("OK");

    delay(5);
    client.stop();
    Serial.println("LED toggled: " + String(channel));
    return;
  }

  // ---------------------------------------------------------
  //  /data  → JSON stream
  // ---------------------------------------------------------
  if (req.startsWith("GET /data")) {

    while (client.available()) {
      String line = client.readStringUntil('\n');
      if (line == "\r" || line.length() == 1) break;
    }

    int values[6];
    for (int i = 0; i < 6; i++) values[i] = analogRead(i);

    client.println("HTTP/1.1 200 OK");
    client.println("Content-Type: application/json");
    client.println("Connection: close");
    client.println();

    client.print("[");
    for (int i = 0; i < 6; i++) {
      client.print(values[i]);
      if (i < 5) client.print(",");
    }
    client.print("]");

    delay(5);
    client.stop();
    Serial.println("client disconnected (data)");
    return;
  }

  // ---------------------------------------------------------
  //  /  → HTML oscilloscope
  // ---------------------------------------------------------
  if (req.startsWith("GET / ")) {

    while (client.available()) {
      String line = client.readStringUntil('\n');
      if (line == "\r" || line.length() == 1) break;
    }

    client.println("HTTP/1.1 200 OK");
    client.println("Content-Type: text/html");
    client.println("Connection: close");
    client.println();

    client.println("<!DOCTYPE HTML>");
    client.println("<html><head><meta charset='UTF-8'>");
    client.println("<title>GIGA Scope</title>");
    client.println("</head><body style='background:#111; color:#ccc; font-family:Bahnschrift, D-DIN, sans-serif;'>");

    client.println("<h2>GIGA R1 WiFi - 6 Channel Scope</h2>");
    client.println("<canvas id='scope' width='900' height='450' style='border:1px solid #444;'></canvas>");
    client.println("<div id='legend' style='margin-top:10px;'></div>");

    // LED buttons
    client.println("<div style='margin-top:15px;'>");
    client.println("<button onclick=\"toggleLED('R')\" style='margin-right:10px;'>Red LED</button>");
    client.println("<button onclick=\"toggleLED('G')\" style='margin-right:10px;'>Green LED</button>");
    client.println("<button onclick=\"toggleLED('B')\">Blue LED</button>");
    client.println("</div>");

    client.println("<script>");
    client.println(R"rawliteral(
const NUM_CHANNELS = 6;
const BUFFER_SIZE = 500;
const MAX_ADC = 1023;

const data = Array.from({length: NUM_CHANNELS}, () => Array(BUFFER_SIZE).fill(0));

const canvas = document.getElementById('scope');
const ctx = canvas.getContext('2d');

canvas.style.background = "#222";

const colors = ["#ff5555","#55ff55","#5599ff","#ffaa33","#cc66ff","#66ffff"];

const marginLeft = 40;
const marginRight = 10;
const marginTop = 10;
const marginBottom = 20;

// LED toggle
async function toggleLED(channel) {
  try {
    await fetch(`/led?c=${channel}`);
  } catch (e) {
    console.log("LED toggle error:", e);
  }
}

function drawGrid() {
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  const w = canvas.width - marginLeft - marginRight;
  const h = canvas.height - marginTop - marginBottom;

  ctx.strokeStyle = "#555";
  ctx.lineWidth = 1;

  const rows = 8;
  for (let i = 0; i <= rows; i++) {
    const y = marginTop + (h * i / rows);
    ctx.beginPath();
    ctx.moveTo(marginLeft, y);
    ctx.lineTo(marginLeft + w, y);
    ctx.stroke();
  }

  const cols = 12;
  for (let i = 0; i <= cols; i++) {
    const x = marginLeft + (w * i / cols);
    ctx.beginPath();
    ctx.moveTo(x, marginTop);
    ctx.lineTo(x, marginTop + h);
    ctx.stroke();
  }

  ctx.strokeStyle = "#888";
  ctx.beginPath();
  ctx.moveTo(marginLeft, marginTop);
  ctx.lineTo(marginLeft, marginTop + h);
  ctx.lineTo(marginLeft + w, marginTop + h);
  ctx.stroke();
}

function drawTickLabels() {
  ctx.fillStyle = "#ccc";
  ctx.font = "12px Bahnschrift, 'D-DIN', sans-serif";
  ctx.textAlign = "right";

  const w = canvas.width - marginLeft - marginRight;
  const h = canvas.height - marginTop - marginBottom;

  const yTicks = [0, 256, 512, 768, 1023];
  for (let v of yTicks) {
    const y = marginTop + h - (v / MAX_ADC) * h;
    ctx.fillText(v.toString(), marginLeft - 8, y + 4);
  }

  ctx.textAlign = "center";
  const xTicks = 5;
  for (let i = 0; i <= xTicks; i++) {
    const x = marginLeft + (w * i / xTicks);
    const label = Math.round((BUFFER_SIZE * i) / xTicks);
    ctx.fillText(label.toString(), x, marginTop + h + 30);
  }
}

function drawAxisLabels() {
  ctx.fillStyle = "#ccc";
  ctx.font = "14px Bahnschrift, 'D-DIN', sans-serif";

  const w = canvas.width - marginLeft - marginRight;
  const h = canvas.height - marginTop - marginBottom;

  ctx.save();
  ctx.translate(15, marginTop + h / 2);
  ctx.rotate(-Math.PI / 2);
  ctx.textAlign = "center";
  ctx.fillText("ADC Value (0–1023)", 0, 0);
  ctx.restore();

  ctx.textAlign = "center";
  ctx.fillText("Time (samples)", marginLeft + w / 2, marginTop + h + 45);
}

function drawTraces() {
  const w = canvas.width - marginLeft - marginRight;
  const h = canvas.height - marginTop - marginBottom;

  ctx.lineWidth = 1.5;

  for (let ch = 0; ch < NUM_CHANNELS; ch++) {
    ctx.strokeStyle = colors[ch];
    ctx.beginPath();

    for (let i = 0; i < BUFFER_SIZE; i++) {
      const x = marginLeft + (i * (w / (BUFFER_SIZE - 1)));
      const v = data[ch][i];
      const y = marginTop + h - (v / MAX_ADC) * h;

      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }

    ctx.stroke();
  }
}

function updateLegend(latestValues) {
  const legend = document.getElementById("legend");
  let html = "";

  for (let ch = 0; ch < NUM_CHANNELS; ch++) {
    html += `
      <span style="
        display:inline-flex;
        align-items:center;
        margin-right:30px;
        font-family:Bahnschrift, 'D-DIN', sans-serif;
        color:#ccc;
      ">
        <span style="
          width:14px;
          height:14px;
          background:${colors[ch]};
          display:inline-block;
          margin-right:6px;
          border-radius:2px;
        "></span>

        <span style="
          width:50px;
          display:inline-block;
        ">
          A${ch}: ${latestValues[ch]}
        </span>
      </span>
    `;
  }

  legend.innerHTML = html;
}

async function poll() {
  try {
    const res = await fetch('/data');
    const vals = await res.json();

    if (!Array.isArray(vals) || vals.length < NUM_CHANNELS) return;

    for (let ch = 0; ch < NUM_CHANNELS; ch++) {
      data[ch].push(vals[ch]);
      if (data[ch].length > BUFFER_SIZE) data[ch].shift();
    }

    drawGrid();
    drawTraces();
    drawTickLabels();
    drawAxisLabels();
    updateLegend(vals);
  } catch (e) {
    console.log("poll error:", e);
  }
}

drawGrid();
drawTraces();
drawTickLabels();
drawAxisLabels();

setInterval(poll, 500);
)rawliteral");
    client.println("</script>");
    client.println("</body></html>");

    delay(5);
    client.stop();
    Serial.println("client disconnected (page)");
    return;
  }

  // ---------------------------------------------------------
  //  Fallback 404
  // ---------------------------------------------------------
  client.println("HTTP/1.1 404 Not Found");
  client.println("Connection: close");
  client.println();
  client.println("Not found");

  delay(5);
  client.stop();
  Serial.println("client disconnected (404)");
}