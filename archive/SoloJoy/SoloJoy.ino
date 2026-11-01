#include <SPI.h>
#include <WiFi.h>
#include <ArduinoWebsockets.h>

using namespace websockets;

// -----------------------------
// WiFi config
// -----------------------------
char ssid[] = "YOUR_WIFI_NAME";
char pass[] = "REDACTED_WIFI_PASSWORD";

WiFiServer httpServer(80);
WebsocketsServer wsServer;     // ArduinoWebsockets 0.5.4 server
WebsocketsClient wsClient;     // Single active client
bool wsActive = false;

// -----------------------------
// Joystick state
// -----------------------------
int  joyAxes[8]     = {0};
bool joyButtons[14] = {0};

// -----------------------------
// Burst buffers
// -----------------------------
const int BURST_SAMPLES = 20;

int axesBuf[8][BURST_SAMPLES];
int btnBuf[14][BURST_SAMPLES];
int analogBuf[6][BURST_SAMPLES];
int scopeBuf[8][BURST_SAMPLES];

int samplesInBurst = 0;

// -----------------------------
// Timing
// -----------------------------
unsigned long lastSampleMs = 0;
unsigned long lastBurstMs  = 0;

// -----------------------------
// Direct Servo PWM on D2
// -----------------------------
const int SERVO_PIN = D2;
volatile int servoPulse = 1500;
unsigned long lastServoMicros = 0;

void updateServoPulseFromAxis(long axisValue) {
  if (axisValue < -32768) axisValue = -32768;
  if (axisValue >  32767) axisValue =  32767;
  servoPulse = map((int)axisValue, -32768, 32767, 1000, 2000);
}

void serviceServo() {
  unsigned long now = micros();
  if (now - lastServoMicros >= 20000) {
    lastServoMicros = now;
    digitalWrite(SERVO_PIN, HIGH);
    delayMicroseconds(servoPulse);
    digitalWrite(SERVO_PIN, LOW);
  }
}

// -----------------------------
// Joystick GET /joy parser
// -----------------------------
void parseJoystick(const String &req) {
  for (int i = 0; i < 8; i++) {
    String key = "ax" + String(i) + "=";
    int idx = req.indexOf(key);
    if (idx > 0) joyAxes[i] = req.substring(idx + key.length()).toInt();
  }

  for (int i = 0; i < 14; i++) {
    String key = "btn" + String(i) + "=";
    int idx = req.indexOf(key);
    if (idx > 0) joyButtons[i] = (req.substring(idx + key.length()).toInt() == 1);
  }

  updateServoPulseFromAxis(joyAxes[0]); // Axis 0 → servo
}

// -----------------------------
// 20 Hz sampling
// -----------------------------
void sampleSignals() {
  unsigned long now = millis();
  if (now - lastSampleMs < 50) return;
  lastSampleMs = now;

  if (samplesInBurst >= BURST_SAMPLES) return;

  int idx = samplesInBurst++;

  for (int a = 0; a < 8; a++) axesBuf[a][idx] = joyAxes[a];
  for (int b = 0; b < 14; b++) btnBuf[b][idx] = joyButtons[b];

  for (int i = 0; i < 6; i++) {
    int v = analogRead(i);
    analogBuf[i][idx] = v;
    scopeBuf[i][idx] = v;
  }

  scopeBuf[6][idx] = 0;
  scopeBuf[7][idx] = 0;
}

// -----------------------------
// 1 Hz WebSocket burst
// -----------------------------
void sendBurstIfReady() {
  if (!wsActive) return;
  if (!wsClient.available()) {   // client still valid?
    wsClient = WebsocketsClient();
    wsActive = false;
    return;
  }

  unsigned long now = millis();
  if (samplesInBurst < BURST_SAMPLES) return;
  if (now - lastBurstMs < 1000) return;
  lastBurstMs = now;

  String json = "{\"burst\":{";

  // axes
  json += "\"axes\":[";
  for (int a = 0; a < 8; a++) {
    json += "[";
    for (int i = 0; i < BURST_SAMPLES; i++) {
      json += axesBuf[a][i];
      if (i < BURST_SAMPLES - 1) json += ",";
    }
    json += "]";
    if (a < 7) json += ",";
  }
  json += "],";

  // buttons
  json += "\"buttons\":[";
  for (int b = 0; b < 14; b++) {
    json += "[";
    for (int i = 0; i < BURST_SAMPLES; i++) {
      json += btnBuf[b][i];
      if (i < BURST_SAMPLES - 1) json += ",";
    }
    json += "]";
    if (b < 13) json += ",";
  }
  json += "],";

  // analog
  json += "\"analog\":[";
  for (int ch = 0; ch < 6; ch++) {
    json += "[";
    for (int i = 0; i < BURST_SAMPLES; i++) {
      json += analogBuf[ch][i];
      if (i < BURST_SAMPLES - 1) json += ",";
    }
    json += "]";
    if (ch < 5) json += ",";
  }
  json += "],";

  // scope
  json += "\"scope\":{";
  for (int ch = 0; ch < 8; ch++) {
    json += "\"ch" + String(ch + 1) + "\":[";
    for (int i = 0; i < BURST_SAMPLES; i++) {
      json += scopeBuf[ch][i];
      if (i < BURST_SAMPLES - 1) json += ",";
    }
    json += "]";
    if (ch < 7) json += ",";
  }
  json += "}";

  json += "}}";

  wsClient.send(json);
  samplesInBurst = 0;
}

// -----------------------------
// HTTP handler (port 80)
// -----------------------------
void handleHttpClient(WiFiClient &client) {
  String req = "";
  bool currentLineIsBlank = true;

  while (client.connected()) {
    if (client.available()) {
      char c = client.read();
      req += c;

      if (req.length() > 1200) break;

      if (c == '\n' && currentLineIsBlank) {

        // /joy (joystick uplink)
        if (req.indexOf("/joy?") >= 0) {
          parseJoystick(req);
          client.println("HTTP/1.1 200 OK");
          client.println("Connection: close");
          client.println();
          client.flush();
          break;
        }

        // Root UI
        client.println("HTTP/1.1 200 OK");
        client.println("Content-Type: text/html");
        client.println("Connection: close");
        client.println();

        client.println("<!DOCTYPE html><html><head>");
        client.println("<meta charset='utf-8'>");
        client.println("<title>Joystick + Burst Scope Monitor</title>");
        client.println("<style>");
        client.println("body{background:#111;color:#eee;font-family:sans-serif;font-size:14px;margin:0;padding:16px;}");
        client.println(".section{margin-bottom:16px;padding:8px;border:1px solid #333;border-radius:4px;}");
        client.println("canvas{background:#111;border:1px solid #333;display:block;margin-top:4px;}");
        client.println(".btn{width:20px;height:20px;display:inline-block;margin:4px;background:#222;border-radius:4px;}");
        client.println(".btn.on{background:#0f0;}");
        client.println("</style>");
        client.println("</head><body>");

        client.println("<div id='wsStatus'>WS: connecting...</div>");
        client.println("<div id='joyStatus'>JOY: idle</div>");
        client.println("<h2>Joystick + Burst Scope Monitor (20 Hz → 1 Hz, 20 s window)</h2>");

        client.println("<div class='section'><h3>Axes</h3><div id='axes'></div></div>");
        client.println("<div class='section'><h3>Buttons</h3><div id='buttons'></div></div>");
        client.println("<div class='section'><h3>Analog Inputs</h3><div id='analogs'></div></div>");
        client.println("<div class='section'><h3>Scope Channels</h3><div id='scope'></div></div>");
        client.println("<div class='section'><h3>Burst info</h3><pre id='burstInfo'></pre></div>");

        client.println("<script>");
        client.println("let axesDiv=document.getElementById('axes');");
        client.println("let btnDiv=document.getElementById('buttons');");
        client.println("let analogDiv=document.getElementById('analogs');");
        client.println("let scopeDiv=document.getElementById('scope');");
        client.println("let wsStatus=document.getElementById('wsStatus');");
        client.println("let joyStatus=document.getElementById('joyStatus');");
        client.println("let burstInfo=document.getElementById('burstInfo');");

        client.println("let gamepadActive=false;");
        client.println("let joyBusy=false;");
        client.println("const HISTORY_LEN=400;");
        client.println("let analogHistory=[[],[],[],[],[],[]];");
        client.println("let scopeHistory=[[],[],[],[],[],[],[],[]];");

        client.println("function buildUI(){");
        client.println("for(let i=0;i<8;i++) axesDiv.innerHTML+=`Axis ${i}: <span id=\"axv${i}\">0</span><br>`;");
        client.println("for(let i=0;i<14;i++) btnDiv.innerHTML+=`<div id=\"btn${i}\" class=\"btn\"></div>`;");
        client.println("for(let i=0;i<6;i++) analogDiv.innerHTML+=`A${i}: <span id=\"an${i}\">0</span><canvas id=\"anPlot${i}\" width=\"300\" height=\"80\"></canvas>`;");
        client.println("for(let i=1;i<=8;i++) scopeDiv.innerHTML+=`CH${i}: <span id=\"sc${i}\">0</span><canvas id=\"scPlot${i}\" width=\"300\" height=\"80\"></canvas>`;");
        client.println("}");

        client.println("function drawMiniScope(id,data){");
        client.println("let c=document.getElementById(id); if(!c||data.length===0) return;");
        client.println("let ctx=c.getContext('2d'); let w=c.width, h=c.height;");
        client.println("let min=Math.min(...data), max=Math.max(...data);");
        client.println("if(min===max){min-=1;max+=1;}");
        client.println("ctx.fillStyle='#111';ctx.fillRect(0,0,w,h);");
        client.println("ctx.strokeStyle='#eee';ctx.beginPath();");
        client.println("for(let i=0;i<data.length;i++){");
        client.println("let x=i*(w/(data.length-1));");
        client.println("let y=h-((data[i]-min)/(max-min))*h;");
        client.println("if(i===0) ctx.moveTo(x,y); else ctx.lineTo(x,y);");
        client.println("}");
        client.println("ctx.stroke();}");

        // WebSocket: note :81 port
        client.println("let ws=new WebSocket('ws://'+location.hostname+':81/ws');");
        client.println("ws.onopen=()=>wsStatus.textContent='WS: connected';");
        client.println("ws.onclose=()=>wsStatus.textContent='WS: disconnected';");
        client.println("ws.onerror=()=>wsStatus.textContent='WS: error';");

        client.println("ws.onmessage=(ev)=>{");
        client.println("let data=JSON.parse(ev.data); let b=data.burst;");
        client.println("if(!b) return;");
        client.println("burstInfo.textContent=JSON.stringify(b,null,2);");
        client.println("let idx=b.axes[0].length-1;");

        client.println("for(let a=0;a<8;a++) document.getElementById('axv'+a).textContent=b.axes[a][idx];");
        client.println("for(let b0=0;b0<14;b0++){");
        client.println("let el=document.getElementById('btn'+b0);");
        client.println("if(b.buttons[b0][idx]) el.classList.add('on'); else el.classList.remove('on');}");

        client.println("for(let ch=0;ch<6;ch++){");
        client.println("let arr=b.analog[ch]; let val=arr[idx];");
        client.println("document.getElementById('an'+ch).textContent=val;");
        client.println("analogHistory[ch].push(...arr);");
        client.println("if(analogHistory[ch].length>HISTORY_LEN) analogHistory[ch].splice(0,analogHistory[ch].length-HISTORY_LEN);");
        client.println("drawMiniScope('anPlot'+ch,analogHistory[ch]);}");

        client.println("for(let ch=1;ch<=8;ch++){");
        client.println("let arr=b.scope['ch'+ch]; let val=arr[idx];");
        client.println("document.getElementById('sc'+ch).textContent=val;");
        client.println("let idx2=ch-1;");
        client.println("scopeHistory[idx2].push(...arr);");
        client.println("if(scopeHistory[idx2].length>HISTORY_LEN) scopeHistory[idx2].splice(0,scopeHistory[idx2].length-HISTORY_LEN);");
        client.println("drawMiniScope('scPlot'+ch,scopeHistory[idx2]);}");

        client.println("};");

        client.println("window.addEventListener('gamepadconnected',()=>{gamepadActive=true;joyStatus.textContent='JOY: connected';});");
        client.println("window.addEventListener('gamepaddisconnected',()=>{gamepadActive=false;joyStatus.textContent='JOY: disconnected';});");
        client.println("window.addEventListener('click',()=>navigator.getGamepads());");

        // Joystick uplink: 1 Hz for now
        client.println("setInterval(()=>{");
        client.println("if(!gamepadActive) return;");
        client.println("if(joyBusy) return;");
        client.println("let gp=navigator.getGamepads()[0];");
        client.println("if(!gp||!gp.connected) return;");
        client.println("joyBusy=true;");
        client.println("joyStatus.textContent='JOY: sending 1 Hz';");
        client.println("let url='/joy?';");
        client.println("for(let i=0;i<8;i++){let v=Math.round((gp.axes[i]||0)*32767);url+=`ax${i}=${v}&`;}");
        client.println("for(let i=0;i<14;i++){let v=(gp.buttons[i]&&gp.buttons[i].pressed)?1:0;url+=`btn${i}=${v}&`;}");
        client.println("fetch(url).finally(()=>joyBusy=false);");
        client.println("},1000);");

        client.println("buildUI();");
        client.println("</script>");

        client.println("</body></html>");
        break;
      }

      if (c == '\n') currentLineIsBlank = true;
      else if (c != '\r') currentLineIsBlank = false;
    }
  }

  client.stop();
}

// -----------------------------
// Setup
// -----------------------------
void setup() {
  Serial.begin(115200);

  pinMode(LED_BLUE, OUTPUT);
  pinMode(SERVO_PIN, OUTPUT);
  digitalWrite(SERVO_PIN, LOW);

  digitalWrite(LED_BLUE, LOW);

  IPAddress ip(192,168,0,50);
  IPAddress gateway(192,168,0,1);
  IPAddress subnet(255,255,255,0);
  WiFi.config(ip, gateway, subnet);

  WiFi.begin(ssid, pass);
  while (WiFi.status() != WL_CONNECTED) delay(100);

  digitalWrite(LED_BLUE, HIGH);

  httpServer.begin();
  wsServer.listen(81);   // ArduinoWebsockets 0.5.4 server on port 81
}

// -----------------------------
// Main loop
// -----------------------------
void loop() {
  serviceServo();

  if (WiFi.status() != WL_CONNECTED) {
    digitalWrite(LED_BLUE, LOW);
    WiFi.begin(ssid, pass);
    delay(100);
    return;
  }

  // Accept WebSocket clients (port 81)
  if (wsServer.poll()) {
    wsClient = wsServer.accept();
    wsActive = true;
  }

  // HTTP handling (port 80)
  WiFiClient client = httpServer.available();
  if (client) handleHttpClient(client);

  sampleSignals();
  sendBurstIfReady();
}