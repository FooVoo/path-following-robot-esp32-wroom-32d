//! ESP32 path-following robot – composition root.
//!
//! This file is intentionally thin.  Its only jobs are:
//!
//! 1. Initialise all ESP32 peripherals.
//! 2. Construct the four hardware adapters (`Drv8833`, `TfLuna` × 2,
//!    `Esp32Joystick`, `WifiAdapter`).
//! 3. Wire them into `Robot` and call `robot.tick(now_ms)` at ~100 Hz.
//!
//! All business logic lives in [`path_following_robot::domain::robot`].
//! All hardware-specific code lives in [`path_following_robot::adapters::esp32`].
//!
//! # Pin assignment (ESP32-WROOM-32D)
//!
//! | Signal     | GPIO | Notes                                              |
//! |------------|------|----------------------------------------------------|
//! | AIN1       |  25  | Left motor forward                                 |
//! | AIN2       |  26  | Left motor reverse                                 |
//! | BIN1       |  32  | Right motor forward                                |
//! | BIN2       |  33  | Right motor reverse                                |
//! | LIDAR L RX |   9  | ⚠ WROOM flash range – remap to GPIO22 if needed   |
//! | LIDAR L TX |  10  | ⚠ WROOM flash range – remap to GPIO23 if needed   |
//! | LIDAR R RX |  16  |                                                    |
//! | LIDAR R TX |  17  |                                                    |
//! | JOY X      |  36  | ADC1 ch0 (VP), input-only, no pull                 |
//! | JOY Y      |  39  | ADC1 ch3 (VN), input-only, no pull                 |
//! | JOY BTN    |  27  | Active-low, internal pull-up                       |
//!
//! # WiFi
//!
//! Set `WIFI_SSID` and `WIFI_PASSWORD` in `src/config.rs` before flashing.
//! The robot broadcasts JSON telemetry to UDP port 9001 and accepts 4-byte
//! command packets on UDP port 9000.  Static IP and gateway are configured
//! in `config.rs` as well.

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is not safe with esp_hal types."
)]
#![deny(clippy::large_stack_frames)]

// Pull in the global allocator supplied by esp-alloc.
// This must appear *before* `esp_hal::init()` so the heap is available for
// the WiFi stack and smoltcp socket buffers.
extern crate alloc;

use core::panic::PanicInfo;

use esp_hal::{
    analog::adc::{Adc, AdcConfig, Attenuation},
    delay::Delay,
    gpio::{DriveMode, Input, InputConfig, Pull},
    interrupt::software::SoftwareInterruptControl,
    ledc::{
        LSGlobalClkSource, LowSpeed, Ledc,
        channel::{self, ChannelIFace},
        timer::{self, TimerIFace},
    },
    main,
    rng::Rng,
    time::{Instant, Rate},
    timer::timg::TimerGroup,
    uart::{Config as UartConfig, UartRx},
    Config,
};
use log::info;

use path_following_robot::{
    adapters::esp32::{
        drv8833::Drv8833, joystick::Esp32Joystick, tf_luna::TfLuna,
        wifi::WifiAdapter,
    },
    config::{LOOP_MS, PWM_FREQ_HZ, WIFI_HEAP_SIZE},
    domain::robot::Robot,
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

    // ── Heap for WiFi stack + smoltcp socket buffers ───────────────────────
    esp_alloc::heap_allocator!(size: WIFI_HEAP_SIZE);

    #[cfg(debug_assertions)]
    esp_println::logger::init_logger(log::LevelFilter::Debug);
    #[cfg(not(debug_assertions))]
    esp_println::logger::init_logger(log::LevelFilter::Info);

    info!("=== path-following-robot booting ===");

    // ── LEDC / Motor PWM ────────────────────────────────────────────────────
    //
    // `timer0` must outlive all channels that hold `&timer0`.
    // Since all locals live for the entire `main()` run, the borrow checker
    // enforces the correct ordering without unsafe code.
    //
    // ChannelIFace + TimerIFace must be in scope for `.configure()` / `.set_duty()`.

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

    // ── UART – TF-Luna LIDARs ───────────────────────────────────────────────
    //
    // ⚠ GPIO9/10 overlap with the WROOM-32D quad-SPI flash.
    //   Remap LIDAR_L to GPIO22/GPIO23 if the board fails to boot.

    let uart_cfg = UartConfig::default();

    let uart_rx_l = UartRx::new(peripherals.UART1, uart_cfg.clone())
        .expect("UART1 init failed")
        .with_rx(peripherals.GPIO9);

    let uart_rx_r = UartRx::new(peripherals.UART2, uart_cfg)
        .expect("UART2 init failed")
        .with_rx(peripherals.GPIO16);

    info!("UART: LIDAR L=UART1/GPIO9  R=UART2/GPIO16");

    let lidar_l = TfLuna::new(uart_rx_l);
    let lidar_r = TfLuna::new(uart_rx_r);

    // ── ADC – Joystick axes ─────────────────────────────────────────────────
    //
    // GPIO36 (VP) / GPIO39 (VN) are input-only; no pull resistor needed.
    // 11 dB attenuation → 0–3.3 V full-scale (12-bit → 0–4095).

    let mut adc_config = AdcConfig::new();
    let joy_x_pin = adc_config.enable_pin(peripherals.GPIO36, Attenuation::_11dB);
    let joy_y_pin = adc_config.enable_pin(peripherals.GPIO39, Attenuation::_11dB);
    let adc1 = Adc::new(peripherals.ADC1, adc_config);

    info!("ADC: joystick X=GPIO36  Y=GPIO39");

    // ── Button ───────────────────────────────────────────────────────────────
    // GPIO27 supports the internal pull-up; active-low.
    let btn = Input::new(
        peripherals.GPIO27,
        InputConfig::default().with_pull(Pull::Up),
    );
    info!("Button: GPIO27 (active-low, internal pull-up)");

    // ── Joystick adapter ─────────────────────────────────────────────────────
    let joystick = Esp32Joystick::new(adc1, joy_x_pin, joy_y_pin, btn);

    // ── WiFi adapter ─────────────────────────────────────────────────────────
    //
    // esp-rtos must be started before esp-radio so that WiFi ISR tasks run in
    // the background and `connect_async()` can resolve.  TIMG1 is used here so
    // that TIMG0 (consumed by LEDC above) is not double-used.
    let timg1 = TimerGroup::new(peripherals.TIMG1);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg1.timer0, sw_int.software_interrupt0);

    let rng = Rng::new();
    let _ = rng; // RNG is no longer passed to WiFi; kept for potential future use.
    let wifi = WifiAdapter::connect(peripherals.WIFI);

    // ── Robot aggregate ──────────────────────────────────────────────────────
    let mut robot = Robot::new_with_wifi(motors, lidar_l, lidar_r, joystick, wifi);

    info!("Robot ready — entering main loop at ~100 Hz");

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
