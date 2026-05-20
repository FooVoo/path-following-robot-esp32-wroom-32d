//! `TelemetryPort` — emit robot-state snapshots over an arbitrary transport.

/// A single telemetry snapshot.
///
/// `state_name` is `&'static str` so that `TelemetryFrame` is `Copy`, heap-free,
/// and safe to use in `#![no_std]` contexts.
///
/// # Note on `robot_ip`
///
/// This struct intentionally omits the robot's IP address.  IP assignment is a
/// network concern, not a domain concern.  The concrete adapter that implements
/// `TelemetryPort` (e.g. `WifiAdapter`) is responsible for injecting the
/// DHCP-assigned IP when encoding the wire frame (e.g. into the
/// `proto::TelemetryFrame::robot_ip` field).  The domain layer never needs to
/// know its own IP.
#[derive(Clone, Copy, Debug)]
pub struct TelemetryFrame {
    /// Current FSM state name: `"IDLE"`, `"RECORD"`, `"READY"`, `"PLAY"`,
    /// `"AVOIDING"`, or `"HALT"`.
    pub state_name: &'static str,
    /// Left LIDAR distance (cm).  `None` if the sensor is stale or absent.
    pub lidar_l_cm: Option<u16>,
    /// Right LIDAR distance (cm).  `None` if the sensor is stale or absent.
    pub lidar_r_cm: Option<u16>,
    /// Left-motor throttle currently applied (`[-100, 100]`).
    pub throttle_l: i8,
    /// Right-motor throttle currently applied (`[-100, 100]`).
    pub throttle_r: i8,
    /// Milliseconds since boot.
    pub uptime_ms: u64,
}

/// Transmit telemetry snapshots over some transport (UDP, UART, BLE, …).
pub trait TelemetryPort {
    /// Send one snapshot.
    ///
    /// Implementations may rate-limit, buffer, or silently discard frames.
    /// The domain calls this every [`TELEMETRY_INTERVAL_MS`] milliseconds.
    ///
    /// [`TELEMETRY_INTERVAL_MS`]: crate::config::TELEMETRY_INTERVAL_MS
    fn send(&mut self, frame: &TelemetryFrame);
}
