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

// -----------------------------
// Joystick Globals
// -----------------------------
float joyAxis[8] = {0};     // 8 axes
bool joyButtons[22] = {0};  // 22 buttons

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

  String req = client.readStringUntil('\r');
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

// /joy  → receive joystick values from browser
if (req.startsWith("GET /joy")) {

  // Axes: ax0=, ax1=, ... ax7=
  for (int i = 0; i < 8; i++) {
    String key = "ax" + String(i) + "=";
    int idx = req.indexOf(key);
    if (idx != -1) {
      joyAxis[i] = req.substring(idx + key.length()).toFloat();
    }
  }

  // Buttons encoded as bitmask (up to 32 bits)
  int ib = req.indexOf("btns=");
  if (ib != -1) {
    unsigned long mask = req.substring(ib + 5).toInt();
    for (int i = 0; i < 22; i++) {
      joyButtons[i] = (mask >> i) & 1;
    }
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
  //  /set?value=X  → receive float from UI
  // ---------------------------------------------------------
  if (req.startsWith("GET /set")) {

    int idx = req.indexOf("value=");
    if (idx != -1) {
      controlValue = req.substring(idx + 6).toFloat();
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
// -----------------------------
// /joydata  → send joystick state to browser
// -----------------------------
if (req.startsWith("GET /joydata")) {

  while (client.available()) {
    String line = client.readStringUntil('\n');
    if (line == "\r" || line.length() == 1) break;
  }

  client.println("HTTP/1.1 200 OK");
  client.println("Content-Type: application/json");
  client.println("Connection: close");
  client.println();

  client.print("{\"axes\":[");
  for (int i = 0; i < 8; i++) {
    client.print(joyAxis[i]);
    if (i < 7) client.print(",");
  }
  client.print("],\"btns\":[");

  for (int i = 0; i < 22; i++) {
    client.print(joyButtons[i] ? 1 : 0);
    if (i < 21) client.print(",");
  }

  client.print("]}");

  client.stop();
  return;
}
// -----------------------------
// Joystick Polling (Browser → Arduino)
// -----------------------------
let lastSend = 0;

function pollGamepad() {
  const pads = navigator.getGamepads();
  const gp = pads[0];
  if (!gp) return;

  // Read up to 8 axes
  const axes = [];
  for (let i = 0; i < 8; i++) {
    axes[i] = gp.axes[i] || 0;
  }

  // Encode up to 22 buttons as bitmask
  let mask = 0;
  for (let i = 0; i < 22; i++) {
    if (gp.buttons[i] && gp.buttons[i].pressed) {
      mask |= (1 << i);
    }
  }

  // Rate limit to 20 Hz
  const now = performance.now();
  if (now - lastSend > 50) {
    const params = axes.map((v, i) => `ax${i}=${v}`).join("&");
    fetch(`/joy?${params}&btns=${mask}`);
    lastSend = now;
  }
}

setInterval(pollGamepad, 20);
 
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
    return;
  }
// -----------------------------
// Joystick UI Update (Arduino → Browser)
// -----------------------------
async function pollJoystickUI() {
  try {
    const res = await fetch('/joydata');
    const j = await res.json();

    // Update 8 axis sliders
    for (let i = 0; i < 8; i++) {
      const el = document.getElementById(`joyAX${i}`);
      if (el) el.value = j.axes[i];
    }

    // Update 22 button LEDs
    let html = "";
    for (let i = 0; i < 22; i++) {
      html += `
        <span style="
          display:inline-block;
          width:20px;
          height:20px;
          border-radius:4px;
          background:${j.btns[i] ? '#0f0' : '#333'};
          border:1px solid #666;
        "></span>
      `;
    }
    document.getElementById("joyButtons").innerHTML = html;

  } catch (e) {}
}

setInterval(pollJoystickUI, 100);

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

client.println("<style>");
client.println("body { background:#111; color:#ccc; font-family:Bahnschrift, D-DIN, sans-serif; margin:20px; }");
client.println("#scope { border:1px solid #444; background:#222; }");
client.println(".tab-bar { margin-bottom:10px; border-bottom:1px solid #333; }");
client.println(".tab-btn { background:#1a1a1a; color:#ccc; border:1px solid #333; border-bottom:none; padding:6px 12px; margin-right:4px; cursor:pointer; font-family:Bahnschrift, D-DIN, sans-serif; font-size:13px; }");
client.println(".tab-btn:hover { background:#2a2a2a; }");
client.println(".tab-btn.active { background:#222; color:#fff; font-weight:bold; border-color:#555; }");
client.println("</style>");

client.println("</head><body>");

//
// FLEX WRAPPER START
//
client.println("<div style='display:flex; gap:20px; align-items:flex-start;'>");

//
// LEFT COLUMN START
//
client.println("<div style='flex:3;' id='leftColumn'>");

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

//
// LEFT COLUMN END
//
client.println("</div>");

//
// RIGHT COLUMN START
//
client.println("<div style='flex:1; background:#1a1a1a; padding:15px; border:1px solid #333; border-radius:6px; font-family:Bahnschrift, D-DIN, sans-serif; color:#ccc;'>");

client.println("<h3 style='margin-top:0;'>Joystick Monitor</h3>");

// 8 axes
for (int i = 0; i < 8; i++) {
  client.print("<div style=\"margin-bottom:10px;\"><label>Axis ");
  client.print(i);
  client.print("</label><input id=\"joyAX");
  client.print(i);
  client.println("\" type=\"range\" min=\"-1\" max=\"1\" step=\"0.01\" value=\"0\" style=\"width:100%;\"></div>");
}

client.println("<h4>Buttons</h4>");
client.println("<div id='joyButtons' style='display:flex; flex-wrap:wrap; gap:6px;'></div>");

//
// RIGHT COLUMN END
//
client.println("</div>");

//
// FLEX WRAPPER END
//
client.println("</div>");

client.println("</body></html>");


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
<script>

// -----------------------------
// Globals
// -----------------------------
const canvas = document.getElementById('scope');
const ctx = canvas.getContext('2d');
const BUFFER_SIZE = 300;
const NUM_CHANNELS = 6;

let data = Array.from({length: NUM_CHANNELS}, () => Array(BUFFER_SIZE).fill(0));
let colors = ['#f33','#3f3','#33f','#ff3','#3ff','#f3f'];

let activeChannel = -1;
let yMin = 0;
let yMax = 1023;
let autoScale = false;

const marginLeft = 50, marginRight = 20, marginTop = 20, marginBottom = 40;

// -----------------------------
// Drawing Functions
// -----------------------------
function drawGrid(){
  ctx.clearRect(0,0,canvas.width,canvas.height);
  const w = canvas.width - marginLeft - marginRight;
  const h = canvas.height - marginTop - marginBottom;

  ctx.strokeStyle='#555'; ctx.lineWidth=1;

  for(let i=0;i<=8;i++){
    let y=marginTop+(h*i/8);
    ctx.beginPath(); ctx.moveTo(marginLeft,y); ctx.lineTo(marginLeft+w,y); ctx.stroke();
  }

  for(let i=0;i<=12;i++){
    let x=marginLeft+(w*i/12);
    ctx.beginPath(); ctx.moveTo(x,marginTop); ctx.lineTo(x,marginTop+h); ctx.stroke();
  }

  ctx.strokeStyle='#888';
  ctx.beginPath();
  ctx.moveTo(marginLeft,marginTop);
  ctx.lineTo(marginLeft,marginTop+h);
  ctx.lineTo(marginLeft+w,marginTop+h);
  ctx.stroke();
}

function drawTickLabels(){
  ctx.fillStyle='#ccc';
  ctx.font='12px Bahnschrift';
  ctx.textAlign='right';

  const w=canvas.width-marginLeft-marginRight;
  const h=canvas.height-marginTop-marginBottom;

  for(let i=0;i<=5;i++){
    let v=yMin+(i*(yMax-yMin)/5);
    let y=marginTop+h-((v-yMin)/(yMax-yMin))*h;
    ctx.fillText(Math.round(v), marginLeft-8, y+4);
  }

  ctx.textAlign='center';
  for(let i=0;i<=5;i++){
    let x=marginLeft+(w*i/5);
    ctx.fillText(Math.round((BUFFER_SIZE*i)/5), x, marginTop+h+30);
  }
}

function drawTraces(){
  const w=canvas.width-marginLeft-marginRight;
  const h=canvas.height-marginTop-marginBottom;
  const range=(yMax-yMin)||1;

  ctx.lineWidth=1.5;

  for(let ch=0;ch<NUM_CHANNELS;ch++){
    if(activeChannel!=-1 && activeChannel!=ch) continue;

    ctx.strokeStyle=colors[ch];
    ctx.beginPath();

    for(let i=0;i<BUFFER_SIZE;i++){
      let x=marginLeft+(i*(w/(BUFFER_SIZE-1)));
      let v=data[ch][i];
      let c=Math.min(Math.max(v,yMin),yMax);
      let y=marginTop+h-((c-yMin)/range)*h;

      if(i==0) ctx.moveTo(x,y);
      else ctx.lineTo(x,y);
    }
    ctx.stroke();
  }
}

function updateLegend(vals){
  let html='';
  for(let ch=0;ch<NUM_CHANNELS;ch++){
    html+=`
      <span style="display:inline-flex;align-items:center;margin-right:30px;color:#ccc;">
        <span style="width:14px;height:14px;background:${colors[ch]};display:inline-block;margin-right:6px;border-radius:2px;"></span>
        <span style="width:70px;display:inline-block;">A${ch}: ${vals[ch]}</span>
      </span>`;
  }
  document.getElementById('legend').innerHTML=html;
}

// -----------------------------
// Polling Loop (data from Arduino)
// -----------------------------
async function poll(){
  try{
    const r=await fetch('/data');
    const j=await r.json();

    for(let ch=0;ch<NUM_CHANNELS;ch++){
      data[ch].shift();
      data[ch].push(j[ch]);
    }

    if(autoScale){
      let all=[];
      for(let ch=0;ch<NUM_CHANNELS;ch++) all=all.concat(data[ch]);
      yMin=Math.min(...all);
      yMax=Math.max(...all);
    }

    drawGrid();
    drawTickLabels();
    drawTraces();
    updateLegend(j);

  }catch(e){}
}
setInterval(poll,50);

// -----------------------------
// Tab System
// -----------------------------
function setChannel(ch,btn){
  activeChannel=ch;
  document.querySelectorAll('.tab-btn').forEach(b=>b.classList.remove('active'));
  btn.classList.add('active');
}

// -----------------------------
// LED Control
// -----------------------------
function toggleLED(c){
  fetch('/led?c='+c);
}

// -----------------------------
// Y-Axis Controls
// -----------------------------
function applyYScale(){
  yMin=parseFloat(document.getElementById('yMin').value);
  yMax=parseFloat(document.getElementById('yMax').value);
  autoScale=false;
}

function enableAutoScale(){
  autoScale=true;
}

// -----------------------------
// Joystick Polling (Browser → Arduino)
// -----------------------------
let lastSend=0;

function pollGamepad(){
  const pads=navigator.getGamepads();
  const gp=pads[0];
  if(!gp) return;

  let axes=[];
  for(let i=0;i<8;i++) axes[i]=gp.axes[i]||0;

  let mask=0;
  for(let i=0;i<22;i++){
    if(gp.buttons[i] && gp.buttons[i].pressed) mask|=(1<<i);
  }

  const now=performance.now();
  if(now-lastSend>50){
    const params=axes.map((v,i)=>`ax${i}=${v}`).join('&');
    fetch(`/joy?${params}&btns=${mask}`);
    lastSend=now;
  }
}
setInterval(pollGamepad,20);

// -----------------------------
// Joystick UI Update (Arduino → Browser)
// -----------------------------
async function pollJoystickUI(){
  try{
    const r=await fetch('/joydata');
    const j=await r.json();

    for(let i=0;i<8;i++){
      let el=document.getElementById(`joyAX${i}`);
      if(el) el.value=j.axes[i];
    }

    let html='';
    for(let i=0;i<22;i++){
      html+=`
        <span style="
          width:20px;
          height:20px;
          border-radius:4px;
          background:${j.btns[i]?'#0f0':'#333'};
          border:1px solid #666;
        "></span>`;
    }
    document.getElementById('joyButtons').innerHTML=html;

  }catch(e){}
}
setInterval(pollJoystickUI,200);

</script>
)rawliteral");
    client.println("</script>");
    client.println("</body></html>");

    delay(5);
    client.stop();
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
}