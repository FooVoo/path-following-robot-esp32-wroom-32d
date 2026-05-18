# Runbook 06 — Hardware Wiring

> **Audience:** Engineers assembling the robot from parts.

---

## 1  Power architecture

The robot uses **two separate power rails**.  Mixing them can brown-out the MCU
or damage components.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                          Power Rails                                         │
│                                                                              │
│  5–9 V  ── Motor battery / power bank                                       │
│            │                                                                 │
│            ├──► DRV8833 VM  (motor supply)                                  │
│            └──► TF-Luna VIN (5 V sensor supply)                             │
│                                                                              │
│  3.3 V  ── ESP32 dev board onboard regulator (from USB or battery via VIN)  │
│            │                                                                 │
│            ├──► ESP32-WROOM-32D VDD                                         │
│            ├──► DRV8833 VCC  (logic supply)                                 │
│            ├──► TF-Luna data lines (3.3 V logic — no level shift needed)    │
│            └──► KY-023 joystick VCC                                         │
│                                                                              │
│  GND ── common ground between all components and both power rails           │
└──────────────────────────────────────────────────────────────────────────────┘
```

> ⚠ **Always connect GND between all power rails.**  A floating ground between
> the motor battery and the logic supply will cause erratic behaviour or
> permanent MCU damage.

---

## 2  DRV8833 motor driver

The DRV8833 is a dual H-bridge.  Each bridge is controlled by two pins:

```
AIN1  AIN2 → Left  motor (Motor A)
BIN1  BIN2 → Right motor (Motor B)

Truth table (fast-decay mode used in firmware):
  AIN1 = PWM duty   AIN2 = 0%  → forward
  AIN1 = 0%         AIN2 = PWM → reverse
  AIN1 = 0%         AIN2 = 0%  → coast (free-wheel)
```

### Wiring

```
ESP32         DRV8833 breakout
──────────────────────────────────────────
GPIO25  ──►  AIN1
GPIO26  ──►  AIN2
GPIO32  ──►  BIN1
GPIO33  ──►  BIN2
3.3 V   ──►  VCC   (logic supply)
GND     ──►  GND
              VM    ◄── 5–9 V motor battery (separate rail)
              AOUT1 ──► Left  motor terminal A
              AOUT2 ──► Left  motor terminal B
              BOUT1 ──► Right motor terminal A
              BOUT2 ──► Right motor terminal B
              nSLEEP ── pull HIGH to 3.3 V (enable driver)
              nFAULT ── optional: pull HIGH, monitor for overcurrent
```

> If the robot drives one wheel backwards unexpectedly, swap the `AOUT1`/`AOUT2`
> wires for that motor (or negate the throttle in `config.rs`).

---

## 3  TF-Luna LIDAR (×2)

The TF-Luna uses **UART at 115 200 baud**.  It **transmits continuously at up
to 100 Hz** — the firmware only needs the RX line.  The TX line (ESP32 → sensor)
is connected but unused in streaming mode.

### Connector pinout (TF-Luna 4-pin JST-GH 1.25 mm)

```
Pin 1: VIN   ── 5 V power supply
Pin 2: GND
Pin 3: TX    ── sensor transmits → ESP32 RX GPIO
Pin 4: RX    ── sensor receives  ← ESP32 TX GPIO (optional in streaming mode)
```

### Wiring — LIDAR left (UART1)

```
TF-Luna L      ESP32
────────────────────────────────
VIN   ──►  5 V rail
GND   ──►  GND
TX    ──►  GPIO9   (UART1 RX)     ⚠ see note below
RX    ◄──  GPIO10  (UART1 TX)
```

### Wiring — LIDAR right (UART2)

```
TF-Luna R      ESP32
────────────────────────────────
VIN   ──►  5 V rail
GND   ──►  GND
TX    ──►  GPIO16  (UART2 RX)
RX    ◄──  GPIO17  (UART2 TX)
```

> ⚠ **GPIO 9/10 conflict:** On the ESP32-WROOM-32D, GPIO 6–11 are internally
> connected to the quad-SPI flash.  Some boards expose GPIO9/10 on headers
> despite this overlap.  If the board **resets on boot** or the LIDAR reads
> are always stale, remap LIDAR-L to the safe pins:
>
> ```rust
> // src/config.rs
> pub const LIDAR_L_RX_GPIO: u8 = 22;
> pub const LIDAR_L_TX_GPIO: u8 = 23;
> ```
>
> Then reconnect the TF-Luna TX → GPIO22 and RX → GPIO23.

---

## 4  KY-023 Joystick

```
KY-023         ESP32
─────────────────────────────────────────────
VCC   ──►  3.3 V
GND   ──►  GND
VRX   ──►  GPIO36  (ADC1 ch0 / VP, input-only)
VRY   ──►  GPIO39  (ADC1 ch3 / VN, input-only)
SW    ──►  GPIO27  (active-low, internal pull-up)
```

GPIO36 and GPIO39 are **input-only** pins — they have no internal pull
resistors and cannot be driven as outputs.  They connect directly to the
joystick potentiometer output (0–3.3 V swing).

GPIO27 supports the internal pull-up.  No external resistor is needed.

> **Calibration:** If the robot drifts slightly at rest, adjust
> `JOY_CENTER_RAW` and `DEAD_ZONE_RAW` in `config.rs` to match your
> joystick's actual centre voltage.

---

## 5  Full wiring summary

```
                           ┌─────────────────────────────────┐
                           │       ESP32-WROOM-32D           │
  ┌────────────────────────┤  3V3 ● GND ● EN ● VP ● VN      │
  │  3.3 V Logic Rail      │                                 │
  │                        │  GPIO25 ──────── DRV8833 AIN1   │
  │  ┌─────────────────────┤  GPIO26 ──────── DRV8833 AIN2   │
  │  │  DRV8833            │  GPIO32 ──────── DRV8833 BIN1   │
  │  │  VM ◄─── 5-9V batt  │  GPIO33 ──────── DRV8833 BIN2   │
  │  │  VCC◄────────────── │  3V3                            │
  │  │  GND ◄──────────── GND                                │
  │  │  AOUT1/2 → L motor  │  GPIO9  ◄────── TF-Luna L TX   │
  │  │  BOUT1/2 → R motor  │  GPIO10 ──────► TF-Luna L RX   │
  │  └─────────────────────│  GPIO16 ◄────── TF-Luna R TX   │
  │                        │  GPIO17 ──────► TF-Luna R RX   │
  │  ┌─────────────────────│  GPIO36(VP) ◄── Joystick VRX   │
  │  │  TF-Luna (both)     │  GPIO39(VN) ◄── Joystick VRY   │
  │  │  VIN ◄──── 5V rail  │  GPIO27 ◄────── Joystick SW    │
  │  │  GND ◄──────────── GND                                │
  │  └─────────────────────│  USB ◄───── USB-UART (flash)   │
  │                        └─────────────────────────────────┘
  │  Joystick               5V rail ─────► TF-Luna VIN (×2)
  │  VCC ◄───────────────── 3.3 V          DRV8833 VM
  │  GND ◄──────────────── GND
  └──────────────────────── 3.3 V
```

---

## 6  Assembly checklist

Before powering on:

- [ ] Common GND connected between logic rail, motor rail, and all sensors
- [ ] DRV8833 VM connected to motor battery (not to 3.3 V)
- [ ] TF-Luna VIN connected to 5 V (not 3.3 V)
- [ ] TF-Luna data lines (TX/RX) connected to correct GPIO RX/TX
- [ ] Joystick SW connected to GPIO27 (not GPIO34–39 which lack pull-up)
- [ ] Motor terminals connected — check polarity with a brief manual test
- [ ] No short circuits between 5 V motor rail and 3.3 V logic rail

After first boot:

- [ ] Serial log shows all peripherals initialised
- [ ] Joystick moves both wheels in correct directions
- [ ] Both LIDAR readings appear in telemetry (not `-1`)
- [ ] Button triggers state transitions as expected
