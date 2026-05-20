//! ESP32 path-following robot — local dev / debug entry point.
//!
//! This binary is intended for **bench testing with a single VL53L0X sensor**
//! (no TCA9548A multiplexer required).  It is otherwise identical to the
//! production firmware except:
//!
//! * **One real VL53L0X** on I2C at `0x29` (direct, no mux).  The left sensor
//!   slot uses the real reading; the right sensor slot is a zero-cost
//!   always-clear stub so the FSM behaves correctly.
//! * **WiFi replaced by [`NoWifi`]** — no network stack, no RTOS, no smoltcp.
//! * **LCD debug output enabled**: the 1602 LCD displays the FSM state on
//!   row 0 and the single-sensor LIDAR distance on row 1.  Updates occur on
//!   every state transition (row 0) and at the telemetry interval (row 1).
//!
//! # Building
//!
//! ```sh
//! cargo +esp build --features dev --bin path-following-robot-dev
//! # or via alias:
//! cargo build-dev
//! ```
//!
//! # Flashing
//!
//! ```sh
//! cargo +esp run --features dev --bin path-following-robot-dev
//! # or:
//! espflash flash --monitor target/xtensa-esp32-none-elf/debug/path-following-robot-dev
//! ```
//!
//! # Pin assignment
//!
//! | Signal       | GPIO      | Notes                                          |
//! |--------------|-----------|------------------------------------------------|
//! | I2C SDA      |  21       | VL53L0X SDA (direct, no mux)                   |
//! | I2C SCL      |  22       | VL53L0X SCL                                    |
//! | AIN1         |  25       | Left motor forward                             |
//! | AIN2         |  26       | Left motor reverse                             |
//! | BIN1         |  32       | Right motor forward                            |
//! | BIN2         |  33       | Right motor reverse                            |
//! | LCD RS       |   5       | HD44780 register select                        |
//! | LCD EN       |   4       | HD44780 enable clock                           |
//! | LCD D4–D7    | 13,14,15,2 | HD44780 data bits (4-bit mode)               |
//! | STEPPER IN1  |  18       | ULN2003 coil A                                 |
//! | STEPPER IN2  |  19       | ULN2003 coil B                                 |
//! | STEPPER IN3  |  23       | ULN2003 coil C                                 |
//! | STEPPER IN4  |  12       | ULN2003 coil D                                 |
//! | JOY X        |  36 (VP)  | ADC1 ch0, input-only                           |
//! | JOY Y        |  39 (VN)  | ADC1 ch3, input-only                           |
//! | JOY BTN      |  27       | Active-low, internal pull-up                   |
//!
//! # LCD debug layout
//!
//! ```text
//! ┌────────────────┐
//! │ IDLE           │  ← row 0: FSM state (updated on every transition)
//! │ L 80 R---  cm  │  ← row 1: lidar readings (updated every 200 ms)
//! └────────────────┘
//! ```
//!
//! Row 0 cycles through: `IDLE` → `RECORD` → `READY` → `PLAY` → `AVOIDING` → `HALT`.
//! In DIRECT mode row 0 shows `DIRECT` for the duration of the session.
//! Row 1 shows `L<dist>` for the real sensor; `R---` because the right slot is a stub.
//!
//! # DIRECT control mode (dev)
//!
//! Hold the joystick button (GPIO27) for ≥ 1 second while the LCD shows `IDLE`
//! to enter **DIRECT** mode.  In DIRECT mode every joystick movement is passed
//! straight to the motors — useful for manually driving the robot on the bench
//! without recording a path.  Press the button once to exit back to `IDLE`.

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is not safe with esp_hal types."
)]
#![deny(clippy::large_stack_frames)]

extern crate alloc;

use core::{cell::RefCell, panic::PanicInfo};

use esp_hal::{
    Config,
    analog::adc::{Adc, AdcConfig, Attenuation},
    delay::Delay,
    gpio::{DriveMode, Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    ledc::{
        LSGlobalClkSource, Ledc, LowSpeed,
        channel::{self, ChannelIFace},
        timer::{self, TimerIFace},
    },
    main,
    time::{Instant, Rate},
};
use log::info;

use path_following_robot::{
    adapters::esp32::{
        drv8833::Drv8833, joystick::Esp32Joystick, lcd1602::Lcd1602, uln2003::Uln2003,
        vl53l0x_direct::Vl53l0xDirect,
    },
    config::{
        I2C_FREQ_HZ, I2C_SCL_GPIO, I2C_SDA_GPIO, LCD_D4_GPIO, LCD_D5_GPIO, LCD_D6_GPIO,
        LCD_D7_GPIO, LCD_EN_GPIO, LCD_RS_GPIO, LOOP_MS, PWM_FREQ_HZ, STEPPER_IN1_GPIO,
        STEPPER_IN2_GPIO, STEPPER_IN3_GPIO, STEPPER_IN4_GPIO,
    },
    domain::robot::{NoWifi, Robot},
    ports::distance::DistancePort,
};

// ── Panic handler ─────────────────────────────────────────────────────────────

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    esp_println::println!("PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}

// ── Application descriptor ───────────────────────────────────────────────────

esp_bootloader_esp_idf::esp_app_desc!();

// ── Single-sensor wrapper ────────────────────────────────────────────────────
//
// `Robot<M, L, I, W, D>` requires both lidar slots to be the same type `L`.
// In dev mode we have one physical sensor and need a clear stub for the other
// slot so the FSM clear-check (`l_clear && r_clear`) does not block PLAY→AVOIDING
// recovery.  `DevSensor` wraps either the real adapter or an always-clear stub.

enum DevSensor<'d> {
    /// Real VL53L0X — reads from the physical sensor at I2C address 0x29.
    Direct(Vl53l0xDirect<'d>),
    /// Always-clear stub — returns `Some(200)` unconditionally.
    ///
    /// Used for the right LIDAR slot so right-side clear checks always pass,
    /// while obstacle avoidance can still be triggered by the real left sensor.
    AlwaysClear,
}

impl<'d> DistancePort for DevSensor<'d> {
    fn poll(&mut self) {
        if let Self::Direct(s) = self {
            s.poll();
        }
    }

    fn distance_cm(&self) -> Option<u16> {
        match self {
            Self::Direct(s) => s.distance_cm(),
            Self::AlwaysClear => Some(200),
        }
    }

    fn tick_staleness(&mut self) {
        if let Self::Direct(s) = self {
            s.tick_staleness();
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[allow(
    clippy::large_stack_frames,
    reason = "PathBuffer (2 KiB) is acceptable in main(); no recursion here."
)]
#[main]
fn main() -> ! {
    // ── Platform init ──────────────────────────────────────────────────────
    let config = Config::default().with_cpu_clock(esp_hal::clock::CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Small heap for alloc types in prost-generated proto code.
    esp_alloc::heap_allocator!(size: 8192);

    #[cfg(debug_assertions)]
    esp_println::logger::init_logger(log::LevelFilter::Debug);
    #[cfg(not(debug_assertions))]
    esp_println::logger::init_logger(log::LevelFilter::Info);

    info!("=== path-following-robot-dev booting (single VL53L0X, LCD debug) ===");

    // ── LEDC / Motor PWM ────────────────────────────────────────────────────
    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

    let mut timer0 = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    timer0
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_hz(PWM_FREQ_HZ),
        })
        .expect("LEDC timer0 configure failed");

    let mut ch_ain1 = ledc.channel::<LowSpeed>(channel::Number::Channel0, peripherals.GPIO25);
    ch_ain1
        .configure(channel::config::Config {
            timer: &timer0,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("LEDC ch_ain1 configure failed");

    let mut ch_ain2 = ledc.channel::<LowSpeed>(channel::Number::Channel1, peripherals.GPIO26);
    ch_ain2
        .configure(channel::config::Config {
            timer: &timer0,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("LEDC ch_ain2 configure failed");

    let mut ch_bin1 = ledc.channel::<LowSpeed>(channel::Number::Channel2, peripherals.GPIO32);
    ch_bin1
        .configure(channel::config::Config {
            timer: &timer0,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("LEDC ch_bin1 configure failed");

    let mut ch_bin2 = ledc.channel::<LowSpeed>(channel::Number::Channel3, peripherals.GPIO33);
    ch_bin2
        .configure(channel::config::Config {
            timer: &timer0,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("LEDC ch_bin2 configure failed");

    info!("LEDC: 4 × 8-bit channels @ {} Hz", PWM_FREQ_HZ);

    // ── Motor adapter ────────────────────────────────────────────────────────
    let motors = Drv8833::new(ch_ain1, ch_ain2, ch_bin1, ch_bin2);

    // ── I2C — single VL53L0X (no mux) ────────────────────────────────────────
    let i2c_cell = RefCell::new(
        I2c::new(
            peripherals.I2C0,
            I2cConfig::default().with_frequency(Rate::from_hz(I2C_FREQ_HZ)),
        )
        .expect("I2C init failed")
        .with_sda(peripherals.GPIO21)
        .with_scl(peripherals.GPIO22),
    );

    let lidar_l = DevSensor::Direct(Vl53l0xDirect::init(&i2c_cell));
    // Right slot: always-clear stub — one physical sensor on the dev bench.
    // The FSM left sensor triggers obstacle detection; the right slot always
    // reports clear so AVOIDING → PLAY recovery is not blocked.
    let lidar_r = DevSensor::AlwaysClear;

    info!(
        "LIDAR: single VL53L0X direct I2C SDA=GPIO{} SCL=GPIO{} @ {}kHz",
        I2C_SDA_GPIO,
        I2C_SCL_GPIO,
        I2C_FREQ_HZ / 1_000
    );

    // ── ADC — Joystick axes ─────────────────────────────────────────────────
    let mut adc_config = AdcConfig::new();
    let joy_x_pin = adc_config.enable_pin(peripherals.GPIO36, Attenuation::_11dB);
    let joy_y_pin = adc_config.enable_pin(peripherals.GPIO39, Attenuation::_11dB);
    let adc1 = Adc::new(peripherals.ADC1, adc_config);

    // ── Button ───────────────────────────────────────────────────────────────
    let btn = Input::new(
        peripherals.GPIO27,
        InputConfig::default().with_pull(Pull::Up),
    );

    // ── Joystick adapter ─────────────────────────────────────────────────────
    let joystick = Esp32Joystick::new(adc1, joy_x_pin, joy_y_pin, btn);
    info!("Joystick: X=GPIO36 (VP)  Y=GPIO39 (VN)  BTN=GPIO27");

    // ── LCD 1602 — debug display ─────────────────────────────────────────────
    //
    // Row 0: FSM state name, updated on every state transition.
    // Row 1: LIDAR distance reading, updated at TELEMETRY_INTERVAL_MS (200 ms).
    //
    // Example:
    //   IDLE
    //   L 80 R---  cm
    let lcd = Lcd1602::new(
        Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default()), // RS
        Output::new(peripherals.GPIO4, Level::Low, OutputConfig::default()), // EN
        Output::new(peripherals.GPIO13, Level::Low, OutputConfig::default()), // D4
        Output::new(peripherals.GPIO14, Level::Low, OutputConfig::default()), // D5
        Output::new(peripherals.GPIO15, Level::Low, OutputConfig::default()), // D6
        Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default()), // D7
    );
    info!(
        "LCD1602 (debug): RS={} EN={} D4-D7={},{},{},{}",
        LCD_RS_GPIO, LCD_EN_GPIO, LCD_D4_GPIO, LCD_D5_GPIO, LCD_D6_GPIO, LCD_D7_GPIO
    );

    // ── ULN2003 stepper driver ────────────────────────────────────────────────
    let _stepper = Uln2003::new(
        Output::new(peripherals.GPIO18, Level::Low, OutputConfig::default()), // IN1
        Output::new(peripherals.GPIO19, Level::Low, OutputConfig::default()), // IN2
        Output::new(peripherals.GPIO23, Level::Low, OutputConfig::default()), // IN3
        Output::new(peripherals.GPIO12, Level::Low, OutputConfig::default()), // IN4
    );
    info!(
        "ULN2003: IN1-IN4={},{},{},{}",
        STEPPER_IN1_GPIO, STEPPER_IN2_GPIO, STEPPER_IN3_GPIO, STEPPER_IN4_GPIO
    );

    // ── Robot aggregate ───────────────────────────────────────────────────────
    let mut robot = Robot::new_full(motors, lidar_l, lidar_r, joystick, NoWifi, lcd);

    info!("Robot ready — dev mode (1 × VL53L0X, LCD debug, no WiFi)");

    // ── Boot reference time ──────────────────────────────────────────────────
    let boot = Instant::now();
    let delay = Delay::new();

    // ── 100 Hz main loop ─────────────────────────────────────────────────────
    loop {
        let now_ms = boot.elapsed().as_millis();
        robot.tick(now_ms);
        delay.delay_millis(LOOP_MS as u32);
    }
}
