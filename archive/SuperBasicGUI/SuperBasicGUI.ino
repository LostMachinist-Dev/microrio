/*
  GIGA R1 WiFi - 6‑channel Web Oscilloscope + External RGB LED Controls
  - External RGB LED header (LED_RED, LED_GREEN, LED_BLUE)
  - Reliable flip‑flop logic using explicit HIGH/LOW writes
  - Parameter control UI sends float to /set?value=X
  - Float stored in global variable and printed to Serial
  - Keysight-style channel tabs with active tab matching chart background (#222)
  - Y-axis manual scaling + auto-scale
*/

#include <SPI.h>
#include <WiFi.h>

char ssid[] = "YOUR_WIFI_NAME";
char pass[] = "REDACTED_WIFI_PASSWORD";
int status = WL_IDLE_STATUS;

WiFiServer server(80);

// External RGB LED states (active‑LOW)
int RState = HIGH;
int GState = HIGH;
int BState = HIGH;

// Global float parameter
float controlValue = 0.0;

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
  while (!Serial) { delay(10); }

  delay(1500);

  // External RGB LED setup (active‑LOW)
  pinMode(LED_RED, OUTPUT);
  pinMode(LED_GREEN, OUTPUT);
  pinMode(LED_BLUE, OUTPUT);

  digitalWrite(LED_RED, HIGH);
  digitalWrite(LED_GREEN, HIGH);
  digitalWrite(LED_BLUE, LOW);

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
  digitalWrite(LED_BLUE, HIGH);
}

// ---------------------------------------------------------
//  Main loop
// ---------------------------------------------------------
void loop() {
  WiFiClient client = server.available();
  if (!client) return;

  Serial.println("new client");

  String req = client.readStringUntil('\r');
  //Serial.println(req);
  client.readStringUntil('\n');

  // ---------------------------------------------------------
  //  /led?c=R|G|B  → toggle external RGB LED (flip‑flop)
  // ---------------------------------------------------------
  if (req.startsWith("GET /led")) {

    char channel = 'X';
    int idx = req.indexOf("c=");
    if (idx != -1 && idx + 2 < req.length()) {
      channel = req.charAt(idx + 2);
    }

    if (channel == 'R') {
      RState = !RState;
      if (RState == LOW) digitalWrite(LED_RED, LOW);
      else digitalWrite(LED_RED, HIGH);
    }

    if (channel == 'G') {
      GState = !GState;
      if (GState == LOW) digitalWrite(LED_GREEN, LOW);
      else digitalWrite(LED_GREEN, HIGH);
    }

    if (channel == 'B') {
      BState = !BState;
      if (BState == LOW) digitalWrite(LED_BLUE, LOW);
      else digitalWrite(LED_BLUE, HIGH);
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
  //  /set?value=X  → receive float from UI
  // ---------------------------------------------------------
  if (req.startsWith("GET /set")) {

    int idx = req.indexOf("value=");
    if (idx != -1) {
      controlValue = req.substring(idx + 6).toFloat();
      Serial.print("Received float: ");
      Serial.println(controlValue);
    }

    client.println("HTTP/1.1 200 OK");
    client.println("Content-Type: text/plain");
    client.println("Connection: close");
    client.println();
    client.println("OK");

    delay(5);
    client.stop();
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
    //Serial.println("client disconnected (data)");
    return;
  }

  // ---------------------------------------------------------
  //  /  → HTML oscilloscope + LED controls + parameter control + tabs + Y-scale
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

    // Basic CSS for dark theme + Keysight-style tabs
    client.println("<style>");
    client.println("body { background:#111; color:#ccc; font-family:Bahnschrift, D-DIN, sans-serif; margin:20px; }");
    client.println("#scope { border:1px solid #444; background:#222; }");
    client.println(".tab-bar { margin-bottom:10px; border-bottom:1px solid #333; }");
    client.println(".tab-btn {");
    client.println("  background:#1a1a1a;");
    client.println("  color:#ccc;");
    client.println("  border:1px solid #333;");
    client.println("  border-bottom:none;");
    client.println("  padding:6px 12px;");
    client.println("  margin-right:4px;");
    client.println("  cursor:pointer;");
    client.println("  font-family:Bahnschrift, D-DIN, sans-serif;");
    client.println("  font-size:13px;");
    client.println("}");
    client.println(".tab-btn:hover { background:#2a2a2a; }");
    client.println(".tab-btn.active {");
    client.println("  background:#222;"); // same as chart background
    client.println("  color:#fff;");
    client.println("  font-weight:bold;");
    client.println("  border-color:#555;");
    client.println("}");
    client.println("</style>");

    client.println("</head><body>");

    client.println("<h2>GIGA R1 WiFi - 6 Channel Scope</h2>");

    // Tab bar
    client.println("<div class='tab-bar'>");
    client.println("<button class='tab-btn active' data-ch='-1' onclick='setChannel(-1, this)'>ALL</button>");
    client.println("<button class='tab-btn' data-ch='0' onclick='setChannel(0, this)'>A0</button>");
    client.println("<button class='tab-btn' data-ch='1' onclick='setChannel(1, this)'>A1</button>");
    client.println("<button class='tab-btn' data-ch='2' onclick='setChannel(2, this)'>A2</button>");
    client.println("<button class='tab-btn' data-ch='3' onclick='setChannel(3, this)'>A3</button>");
    client.println("<button class='tab-btn' data-ch='4' onclick='setChannel(4, this)'>A4</button>");
    client.println("<button class='tab-btn' data-ch='5' onclick='setChannel(5, this)'>A5</button>");
    client.println("</div>");

    client.println("<canvas id='scope' width='900' height='450'></canvas>");
    client.println("<div id='legend' style='margin-top:10px;'></div>");

    // Y-axis scale controls
    client.println(R"rawliteral(
<div style="margin-top:15px; color:#ccc; font-family:Bahnschrift, D-DIN, sans-serif;">
  <h3>Y-Axis Scale</h3>
  <div style="display:flex; align-items:center; gap:10px;">
    <span>Min:</span>
    <input id="yMin" type="number" step="1" value="0" style="width:80px; padding:4px;">
    <span>Max:</span>
    <input id="yMax" type="number" step="1" value="1023" style="width:80px; padding:4px;">
    <button onclick="applyYScale()" style="margin-left:10px;">Apply</button>
    <button onclick="enableAutoScale()" style="margin-left:10px;">Auto-Scale</button>
  </div>
</div>
)rawliteral");

    // LED buttons
    client.println("<div style='margin-top:15px;'>");
    client.println("<button onclick=\"toggleLED('R')\" style='margin-right:10px;'>Red LED</button>");
    client.println("<button onclick=\"toggleLED('G')\" style='margin-right:10px;'>Green LED</button>");
    client.println("<button onclick=\"toggleLED('B')\">Blue LED</button>");
    client.println("</div>");

    // Parameter control UI
    client.println(R"rawliteral(
<div style="margin-top:20px; color:#ccc; font-family:Bahnschrift, D-DIN, sans-serif;">
  <h3>
    Parameter Control
    <span id="paramEcho" style="margin-left:15px; font-weight:normal;">
      Param: 0.000
    </span>
  </h3>

  <div style="display:flex; align-items:center; gap:10px;">

    <!-- Main value -->
    <input id="paramValue" type="number" step="0.01" value="0.00"
           style="width:80px; padding:4px;">

    <!-- Up/Down arrows -->
    <div style="display:flex; flex-direction:column;">
      <button onclick="adjustValue(1)" style="width:30px;">▲</button>
      <button onclick="adjustValue(-1)" style="width:30px;">▼</button>
    </div>

    <!-- Step size -->
    <span>Step:</span>
    <input id="paramStep" type="number" step="0.01" value="0.10"
           style="width:60px; padding:4px;">

    <!-- Min -->
    <span>Min:</span>
    <input id="paramMin" type="number" step="0.01" value="-10.00"
           style="width:60px; padding:4px;">

    <!-- Max -->
    <span>Max:</span>
    <input id="paramMax" type="number" step="0.01" value="10.00"
           style="width:60px; padding:4px;">

    <!-- Manual send button -->
    <button onclick="sendValue()" style="margin-left:10px;">Send</button>
  </div>
</div>
)rawliteral");

    // JavaScript
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

// -----------------------------
// Y-axis scaling
// -----------------------------
let yMin = 0;
let yMax = 1023;
let autoScale = false;

function applyYScale() {
  autoScale = false;
  let minVal = parseFloat(document.getElementById("yMin").value);
  let maxVal = parseFloat(document.getElementById("yMax").value);

  if (isNaN(minVal)) minVal = 0;
  if (isNaN(maxVal)) maxVal = 1023;

  if (maxVal <= minVal) {
    minVal = 0;
    maxVal = 1023;
  }

  yMin = minVal;
  yMax = maxVal;

  drawGrid();
  drawTraces();
  drawTickLabels();
  drawAxisLabels();
}

function enableAutoScale() {
  autoScale = true;
}

// -----------------------------
// Channel Tabs
// -----------------------------
let activeChannel = -1;  // -1 = ALL

function setChannel(ch, btn) {
  activeChannel = ch;

  // Update tab button active state
  const tabs = document.querySelectorAll('.tab-btn');
  tabs.forEach(t => t.classList.remove('active'));
  if (btn) btn.classList.add('active');
  else {
    // Fallback: match by data-ch if btn not passed
    tabs.forEach(t => {
      if (t.getAttribute('data-ch') == String(ch)) t.classList.add('active');
    });
  }

  drawGrid();
  drawTraces();
  drawTickLabels();
  drawAxisLabels();
}

// -----------------------------
// LED toggle
// -----------------------------
async function toggleLED(channel) {
  try {
    await fetch(`/led?c=${channel}`);
  } catch (e) {
    console.log("LED toggle error:", e);
  }
}

// -----------------------------
// Parameter Control
// -----------------------------
function updateParamEcho(value) {
  const echo = document.getElementById("paramEcho");
  if (echo) echo.textContent = `Param: ${value.toFixed(3)}`;
}

async function adjustValue(direction) {
  let value = parseFloat(document.getElementById("paramValue").value);
  let step  = parseFloat(document.getElementById("paramStep").value);
  let min   = parseFloat(document.getElementById("paramMin").value);
  let max   = parseFloat(document.getElementById("paramMax").value);

  if (isNaN(value)) value = 0;
  if (isNaN(step)) step = 1;
  if (isNaN(min)) min = -Infinity;
  if (isNaN(max)) max = Infinity;

  value += direction * step;

  if (value < min) value = min;
  if (value > max) value = max;

  document.getElementById("paramValue").value = value.toFixed(3);
  updateParamEcho(value);

  try {
    await fetch(`/set?value=${value}`);
    console.log("Sent value (adjust):", value);
  } catch (e) {
    console.log("Send error (adjust):", e);
  }
}

async function sendValue() {
  let value = parseFloat(document.getElementById("paramValue").value);
  if (isNaN(value)) value = 0;

  updateParamEcho(value);

  try {
    await fetch(`/set?value=${value}`);
    console.log("Sent value (manual):", value);
  } catch (e) {
    console.log("Send error (manual):", e);
  }
}

// -----------------------------
// Drawing Functions
// -----------------------------
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

  const steps = 5;
  for (let i = 0; i <= steps; i++) {
    const v = yMin + (i * (yMax - yMin) / steps);
    const y = marginTop + h - ((v - yMin) / (yMax - yMin)) * h;
    const label = Math.round(v);
    ctx.fillText(label.toString(), marginLeft - 8, y + 4);
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
  ctx.fillText("ADC Value", 0, 0);
  ctx.restore();

  ctx.textAlign = "center";
  ctx.fillText("Time (samples)", marginLeft + w / 2, marginTop + h + 45);
}

function drawTraces() {
  const w = canvas.width - marginLeft - marginRight;
  const h = canvas.height - marginTop - marginBottom;

  ctx.lineWidth = 1.5;

  const range = (yMax - yMin) || 1;

  for (let ch = 0; ch < NUM_CHANNELS; ch++) {

    // Only draw selected channel
    if (activeChannel !== -1 && activeChannel !== ch) continue;

    ctx.strokeStyle = colors[ch];
    ctx.beginPath();

    for (let i = 0; i < BUFFER_SIZE; i++) {
      const x = marginLeft + (i * (w / (BUFFER_SIZE - 1)));
      const v = data[ch][i];
      const clamped = Math.min(Math.max(v, yMin), yMax);
      const y = marginTop + h - ((clamped - yMin) / range) * h;

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

        <span style="width:50px; display:inline-block;">
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

    // Auto-scale if enabled
    if (autoScale) {
      let minV = Infinity;
      let maxV = -Infinity;

      for (let ch = 0; ch < NUM_CHANNELS; ch++) {
        for (let i = 0; i < BUFFER_SIZE; i++) {
          const v = data[ch][i];
          if (v < minV) minV = v;
          if (v > maxV) maxV = v;
        }
      }

      if (!isFinite(minV) || !isFinite(maxV)) {
        minV = 0;
        maxV = 1023;
      }

      // Add padding
      minV -= 10;
      maxV += 10;

      if (minV < 0) minV = 0;
      if (maxV > MAX_ADC) maxV = MAX_ADC;
      if (maxV <= minV) {
        minV = 0;
        maxV = MAX_ADC;
      }

      yMin = minV;
      yMax = maxV;

      // Reflect in UI
      const yMinInput = document.getElementById("yMin");
      const yMaxInput = document.getElementById("yMax");
      if (yMinInput) yMinInput.value = Math.round(yMin);
      if (yMaxInput) yMaxInput.value = Math.round(yMax);
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

setInterval(poll, 250); //!!!UPDATE THIS TO CHANGE THE BROWSER POLL RATE!!!
)rawliteral");
    client.println("</script>");
    client.println("</body></html>");

    delay(5);
    client.stop();
    //Serial.println("client disconnected (page)");
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