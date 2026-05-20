//! ESP32 path-following robot – Wokwi simulation entry point.
//!
//! Identical to `main.rs` except:
//!
//! * I2C bus, TCA9548A mux, and VL53L0X LIDARs are replaced by two
//!   [`StubDistance`] adapters (oscillating fixed values — no I2C hardware
//!   required, and TCA9548A is not supported by Wokwi).
//! * WiFi is replaced by [`NoWifi`] (silent no-ops — no RTOS, no smoltcp).
//! * The stepper is still wired up but not driven by the FSM.
//!
//! # Building
//!
//! ```sh
//! cargo +esp build --features sim --bin path-following-robot-sim
//! # or via alias:
//! cargo build-sim
//! ```
//!
//! # Running in Wokwi
//!
//! See `docs/runbooks/09-simulation.md`.
//!
//! # Pin assignment (same as `main.rs`)
//!
//! | Signal       | GPIO      | Notes                                          |
//! |--------------|-----------|------------------------------------------------|
//! | AIN1         |  25       | Left motor forward (visualised, no feedback)   |
//! | AIN2         |  26       | Left motor reverse                             |
//! | BIN1         |  32       | Right motor forward                            |
//! | BIN2         |  33       | Right motor reverse                            |
//! | LCD RS       |   5       | HD44780 register select                        |
//! | LCD EN       |   4       | HD44780 enable clock                           |
//! | LCD D4–D7    | 13,14,15,2 | HD44780 data bits (4-bit mode)               |
//! | STEPPER IN1  |  18       | ULN2003 coil A (not FSM-driven in sim)         |
//! | STEPPER IN2  |  19       | ULN2003 coil B                                 |
//! | STEPPER IN3  |  23       | ULN2003 coil C                                 |
//! | STEPPER IN4  |  12       | ULN2003 coil D                                 |
//! | JOY X        |  36 (VP)  | ADC1 ch0, input-only, no pull                  |
//! | JOY Y        |  39 (VN)  | ADC1 ch3, input-only, no pull                  |
//! | JOY BTN      |  27       | Active-low, internal pull-up                   |

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is not safe with esp_hal types."
)]
#![deny(clippy::large_stack_frames)]

// Pull in the global allocator — required because lib.rs includes `pub mod proto`
// which contains prost-generated code that uses `alloc::string::String`.
// WiFi / smoltcp are disabled, so a tiny heap (8 KiB) is sufficient.
extern crate alloc;

use core::panic::PanicInfo;

use esp_hal::{
    Config,
    analog::adc::{Adc, AdcConfig, Attenuation},
    delay::Delay,
    gpio::{DriveMode, Input, InputConfig, Level, Output, OutputConfig, Pull},
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
    adapters::{
        esp32::{drv8833::Drv8833, joystick::Esp32Joystick, lcd1602::Lcd1602, uln2003::Uln2003},
        stub::StubDistance,
    },
    config::{
        LCD_D4_GPIO, LCD_D5_GPIO, LCD_D6_GPIO, LCD_D7_GPIO, LCD_EN_GPIO, LCD_RS_GPIO, LOOP_MS,
        PWM_FREQ_HZ, STEPPER_IN1_GPIO, STEPPER_IN2_GPIO, STEPPER_IN3_GPIO, STEPPER_IN4_GPIO,
    },
    domain::robot::{NoWifi, Robot},
};

// ── Panic handler ─────────────────────────────────────────────────────────────

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    esp_println::println!("PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}

// ── Application descriptor (required by the esp-idf bootloader) ───────────────

esp_bootloader_esp_idf::esp_app_desc!();

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

    // Small heap — only needed for alloc types in prost-generated proto code.
    // WiFi / smoltcp disabled, so 8 KiB is more than sufficient.
    esp_alloc::heap_allocator!(size: 8192);

    #[cfg(debug_assertions)]
    esp_println::logger::init_logger(log::LevelFilter::Debug);
    #[cfg(not(debug_assertions))]
    esp_println::logger::init_logger(log::LevelFilter::Info);

    info!("=== path-following-robot-sim booting (Wokwi) ===");

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

    // ── Stub LIDAR sensors ───────────────────────────────────────────────────
    // Replace the real VL53L0X-on-TCA9548A pair with oscillating stubs.
    // TCA9548A is unsupported by Wokwi; no I2C wiring in diagram.json.
    // Cycle: 400 ticks at 200 cm (safe) → 100 ticks at 50 cm (obstacle).
    let lidar_l = StubDistance::new();
    let lidar_r = StubDistance::new();
    info!("LIDAR: StubDistance (200 cm × 4 s → 50 cm × 1 s, repeating)");

    // ── ADC – Joystick axes ─────────────────────────────────────────────────
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

    // ── LCD 1602 (HD44780, 4-bit parallel) ──────────────────────────────────
    let lcd = Lcd1602::new(
        Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default()), // RS
        Output::new(peripherals.GPIO4, Level::Low, OutputConfig::default()), // EN
        Output::new(peripherals.GPIO13, Level::Low, OutputConfig::default()), // D4
        Output::new(peripherals.GPIO14, Level::Low, OutputConfig::default()), // D5
        Output::new(peripherals.GPIO15, Level::Low, OutputConfig::default()), // D6
        Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default()), // D7
    );
    info!(
        "LCD1602: RS={} EN={} D4-D7={},{},{},{}",
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

    // ── Robot aggregate (WiFi disabled) ──────────────────────────────────────
    let mut robot = Robot::new_full(motors, lidar_l, lidar_r, joystick, NoWifi, lcd);

    info!("Robot ready — main loop at ~100 Hz (WiFi disabled in sim)");

    // ── Boot reference time ──────────────────────────────────────────────────
    let boot = Instant::now();
    let delay = Delay::new();

    // ── Cooperative 100 Hz main loop ─────────────────────────────────────────
    loop {
        let now_ms = boot.elapsed().as_millis();
        robot.tick(now_ms);
        delay.delay_millis(LOOP_MS as u32);
    }
}
