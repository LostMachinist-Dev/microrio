#include <Wire.h>
#include <RTClib.h>
#include <Ethernet.h>
#include <SPI.h>
#include <TelemOverETH.h>

RTC_DS3231 rtc;
TelemOverETH telem;

// ----------------------
// Network Configuration
// ----------------------
byte mac[] = { 0xDE,0xAD,0xBE,0xEF,0xFE,0xED };

IPAddress ip(192,168,0,50);
IPAddress gateway(192,168,0,1);
IPAddress subnet(255,255,255,0);
IPAddress dns(192,168,0,1);

// ----------------------
// Setup
// ----------------------
void setup() {
    Wire.begin();
    rtc.begin();

    telem.ipsetup(mac, ip, gateway, subnet, dns);
    telem.freq(10);          // 10 Hz telemetry
    telem.max_val(128);      // plenty of keys
    telem.begin();

    pinMode(LED_BUILTIN, OUTPUT);
}

// ----------------------
// Loop
// ----------------------
void loop() {
    telem.loop();

    // Read analog
    int a0 = analogRead(A0);
    telem.write("a0", a0);

    digitalWrite(LED_BUILTIN, telem.readBool("led"));

    // RTC time
    DateTime now = rtc.now();

    // Human-readable time
    char timestr[12];
    snprintf(timestr, sizeof(timestr),
             "%02d:%02d:%02d",
             now.hour(), now.minute(), now.second());
    telem.write("rtc_time", timestr);

    // --- Operator time sync ---
    if (telem.readBool("sync_time")) {
        long newUnix = telem.readLong("unix_set");
        if (newUnix > 1000000000L) {   // sanity check
            rtc.adjust(DateTime(newUnix));
        }
        telem.write("sync_time", false);   // one-shot
    }

    // UNIX time
    telem.write("unix", (long)now.unixtime());

    // Loop duty cycle
    telem.write("loop_pct", telem.loopDuty());
}

