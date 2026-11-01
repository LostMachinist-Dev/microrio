#ifndef REMOTE_IO_H
#define REMOTE_IO_H

#include <Arduino.h>
#include <Ethernet.h>

// ============================================================
//  RemoteIO — Minimal, deterministic telemetry/control library
// ============================================================
//
//  Telemetry capacity:
//    - 128 bools
//    - 64 ints (int32_t)
//    - 32 doubles (double)
//    - 12 strings (32 chars each)
//
//  Control capacity mirrors telemetry.
//
//  Telemetry is sent in 4 packets per frame:
//    Packet 0: bools + timestamp
//    Packet 1: ints
//    Packet 2: doubles
//    Packet 3: strings
//
//  Control is received in the same 4‑packet structure.
//
//  This header exposes only the public API.
//  All packet logic lives in RemoteIO.cpp.
//
// ============================================================

class RemoteIO {
public:

    // ------------------------------------------------------------
    // Initialization
    // ------------------------------------------------------------
    void begin(IPAddress controllerIp,
               IPAddress consoleIp,
               uint16_t controllerPort,
               uint16_t consolePort);

    // ------------------------------------------------------------
    // Optional time sources (user-provided)
    // ------------------------------------------------------------
    void setTimeSourceSeconds(uint32_t (*fn)());
    void setTimeSourceMicros(uint32_t (*fn)());

    // ------------------------------------------------------------
    // Telemetry rate (Hz)
    // ------------------------------------------------------------
    void setRate(uint8_t hz);

    // ------------------------------------------------------------
    // Poll incoming control packets + send telemetry if due
    // ------------------------------------------------------------
    void poll();

    // ------------------------------------------------------------
    // Control getters
    // ------------------------------------------------------------
    bool      controlBool(uint8_t id)   const;
    int32_t   controlInt(uint8_t id)    const;
    double    controlDouble(uint8_t id) const;
    String    controlStr(uint8_t id)    const;

    // ------------------------------------------------------------
    // Telemetry setters
    // ------------------------------------------------------------
    void telemBool(uint8_t id, bool value);
    void telemInt(uint8_t id, int32_t value);
    void telemDouble(uint8_t id, double value);
    void telemStr(uint8_t id, const String& value);

    // ------------------------------------------------------------
    // Force immediate telemetry send (optional)
    // ------------------------------------------------------------
    void sendNow();

    // ------------------------------------------------------------
    // Bulk accessors for SD logging (read-only)
    // ------------------------------------------------------------
    const bool*     telemBoolArray()   const;
    const int32_t*  telemIntArray()    const;
    const double*   telemDoubleArray() const;
    const String*   telemStrArray()    const;

    const bool*     controlBoolArray()   const;
    const int32_t*  controlIntArray()    const;
    const double*   controlDoubleArray() const;
    const String*   controlStrArray()    const;

    // Counts for logging
    uint16_t telemBoolCount()   const { return 128; }
    uint16_t telemIntCount()    const { return 64; }
    uint16_t telemDoubleCount() const { return 32; }
    uint16_t telemStrCount()    const { return 12; }

    uint16_t controlBoolCount()   const { return 128; }
    uint16_t controlIntCount()    const { return 64; }
    uint16_t controlDoubleCount() const { return 32; }
    uint16_t controlStrCount()    const { return 12; }

    // ------------------------------------------------------------
    // Control timestamp (from incoming packet 0)
    // ------------------------------------------------------------
    uint32_t controlTimestampSec()  const { return lastControlSec; }
    uint32_t controlTimestampUSec() const { return lastControlUSec; }

private:

    // Networking
    EthernetUDP udpIn;
    EthernetUDP udpOut;

    IPAddress controllerIP;
    IPAddress consoleIP;
    uint16_t controllerPort;
    uint16_t consolePort;

    // Telemetry scheduling
    uint8_t  rateHz = 25;
    uint32_t nextSendMicros = 0;

    // Optional time sources
    uint32_t (*timeSecFn)()  = nullptr;
    uint32_t (*timeUSecFn)() = nullptr;

    // Control timestamp (from packet 0)
    uint32_t lastControlSec  = 0;
    uint32_t lastControlUSec = 0;

    // Control storage
    bool     ctrlBoolArr[128];
    int32_t  ctrlIntArr[64];
    double   ctrlDoubleArr[32];
    String   ctrlStrArr[12];

    // Telemetry storage
    bool     telemBoolArr[128];
    int32_t  telemIntArr[64];
    double   telemDoubleArr[32];
    String   telemStrArr[12];

    // Internal helpers
    void sendTelemetryIfDue();
    void parseIncomingPacket();

    void sendPacketBools();
    void sendPacketInts();
    void sendPacketDoubles();
    void sendPacketStrings();
};

#endif