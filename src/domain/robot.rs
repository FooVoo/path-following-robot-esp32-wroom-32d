//! Pure-domain robot aggregate.
//!
//! `Robot<M, L, I, W>` implements the seven-state FSM without any direct
//! dependency on `esp_hal`.  All timing uses `now_ms: u64` (milliseconds
//! since boot) passed in by the caller.  Hardware access is mediated
//! exclusively through the four port traits:
//!
//! * [`MotorPort`]         — drive the DRV8833 motor driver
//! * [`DistancePort`]      — read TF-Luna LIDAR sensors
//! * [`InputPort`]         — sample the joystick and button
//! * [`RemoteControlPort`] + [`TelemetryPort`] (via `W`) — WiFi remote / telemetry
//!
//! The `W` generic defaults to [`NoWifi`] (silent no-ops), so all existing
//! callers using `Robot::new(…)` with four arguments compile unchanged.
//!
//! # State machine
//!
//! ```text
//!  ┌────────┐  short press  ┌────────┐  button  ┌───────┐  button  ┌──────┐
//!  │  IDLE  │ ────────────► │ RECORD │ ───────► │ READY │ ───────► │ PLAY │
//!  │        │               └────────┘          └───────┘          └──┬───┘
//!  │        │  long press   ┌────────┐                     obstacle    │
//!  │        │ ────────────► │ DIRECT │                               ▼
//!  └────────┘◄── button ─── └────────┘                       ┌──────────┐
//!                                                             │ AVOIDING │
//!                buffer full / path done                      └──────────┘
//!                      │                                           │
//!                      ▼        path done / avoidance timeout      │
//!                  ┌──────┐ ◄───────────────────────────────────────
//!                  │ HALT │
//!                  └──────┘
//! ```

use log::{debug, error, info, trace, warn};

use crate::{
    config::{
        AVOID_BACK_MS, AVOID_TIMEOUT_MS, AVOID_TURN_MS, CLEAR_CM, HALT_LOG_INTERVAL_MS,
        LONG_PRESS_MS, OBSTACLE_CM, PATH_CMD_INTERVAL_MS, TELEMETRY_INTERVAL_MS, WARN_CM,
    },
    domain::{
        path::{PathBuffer, PathCommand},
        state::{ObstacleSide, RobotState},
    },
    ports::{
        display::DisplayPort,
        distance::DistancePort,
        input::InputPort,
        motors::MotorPort,
        remote_control::RemoteControlPort,
        telemetry::{TelemetryFrame, TelemetryPort},
    },
};

// ── No-op WiFi implementation ─────────────────────────────────────────────────

/// No-op WiFi port — used as the default `W` type when WiFi is disabled or
/// in unit tests.
///
/// Both [`RemoteControlPort`] and [`TelemetryPort`] are implemented as silent
/// no-ops so that `Robot::new()` (4-argument form) compiles without any WiFi
/// dependencies.
pub struct NoWifi;

impl RemoteControlPort for NoWifi {
    fn poll_network(&mut self, _now_ms: u64) {}
    fn poll_throttle(&mut self) -> Option<(i8, i8)> { None }
    fn poll_button(&mut self) -> bool { false }
}

impl TelemetryPort for NoWifi {
    fn send(&mut self, _frame: &TelemetryFrame) {}
}

// ── No-op Display implementation ──────────────────────────────────────────────

/// No-op display port — used as the default `D` type when no LCD is connected.
///
/// Both [`DisplayPort`] methods are silent no-ops so that `Robot::new()` and
/// `Robot::new_with_wifi()` compile without any display dependency.
pub struct NoDisplay;

impl DisplayPort for NoDisplay {
    fn print_row(&mut self, _row: u8, _text: &str) {}
    fn clear(&mut self) {}
}

// ── Robot aggregate ───────────────────────────────────────────────────────────

/// The robot aggregate — owns ports and state machine data.
///
/// Generic parameters:
/// * `M` — motor port (DRV8833 adapter)
/// * `L` — distance port (TF-Luna / VL53L0X adapter; same type for both sensors)
/// * `I` — input port  (joystick adapter)
/// * `W` — WiFi port implementing both [`RemoteControlPort`] and
///          [`TelemetryPort`]; defaults to [`NoWifi`] (silent no-ops).
/// * `D` — display port implementing [`DisplayPort`]; defaults to [`NoDisplay`]
///          (silent no-ops).
pub struct Robot<M, L, I, W = NoWifi, D = NoDisplay> {
    motors: M,
    lidar_l: L,
    lidar_r: L,
    input: I,
    /// Remote WiFi port — polled once per tick for incoming commands and
    /// used to emit telemetry at `TELEMETRY_INTERVAL_MS` intervals.
    wifi: W,
    /// Character display — updated on state change (row 0) and at the
    /// telemetry cadence (row 1, lidar readings).
    display: D,

    state: RobotState,
    path: PathBuffer,

    /// `now_ms` when the current record segment started.
    record_start_ms: u64,
    /// Index into `path` of the command currently being replayed.
    play_idx: usize,
    /// `now_ms` when the current playback command started.
    cmd_start_ms: u64,
    /// `now_ms` when avoidance manoeuvre started.
    avoid_start_ms: u64,
    /// Which side triggered avoidance.
    avoid_side: Option<ObstacleSide>,
    /// `now_ms` of the last HALT log line (rate-limiting).
    halt_log_ms: Option<u64>,

    /// Last throttle values sent to motors — tracked for telemetry frames.
    last_tl: i8,
    last_tr: i8,
    /// `now_ms` when the last telemetry frame was emitted.
    last_telemetry_ms: u64,
    /// State displayed on row 0 at the last display update; prevents redundant writes.
    last_display_state: RobotState,
    /// `now_ms` when the physical button was first detected as held in IDLE.
    ///
    /// `None` when the button is not pressed.  Used to distinguish a long press
    /// (≥ `LONG_PRESS_MS` → DIRECT) from a short press (< `LONG_PRESS_MS` → RECORD).
    button_hold_start: Option<u64>,
}

// ── Constructors ──────────────────────────────────────────────────────────────

impl<M, L, I> Robot<M, L, I, NoWifi, NoDisplay>
where
    M: MotorPort,
    L: DistancePort,
    I: InputPort,
{
    /// Construct the robot with WiFi and display both disabled (all no-ops).
    ///
    /// This is the standard constructor used in tests and when neither a WiFi
    /// adapter nor an LCD is wired up.
    pub fn new(motors: M, lidar_l: L, lidar_r: L, input: I) -> Self {
        Robot::new_full(motors, lidar_l, lidar_r, input, NoWifi, NoDisplay)
    }
}

impl<M, L, I, W> Robot<M, L, I, W, NoDisplay>
where
    M: MotorPort,
    L: DistancePort,
    I: InputPort,
    W: RemoteControlPort + TelemetryPort,
{
    /// Construct the robot with a WiFi adapter but no display.
    ///
    /// `wifi` must implement both [`RemoteControlPort`] (receives commands) and
    /// [`TelemetryPort`] (sends state snapshots).
    pub fn new_with_wifi(motors: M, lidar_l: L, lidar_r: L, input: I, wifi: W) -> Self {
        Robot::new_full(motors, lidar_l, lidar_r, input, wifi, NoDisplay)
    }
}

impl<M, L, I, W, D> Robot<M, L, I, W, D>
where
    M: MotorPort,
    L: DistancePort,
    I: InputPort,
    W: RemoteControlPort + TelemetryPort,
    D: DisplayPort,
{
    /// Construct the robot with all five ports explicitly specified.
    ///
    /// This is the full constructor used by `main.rs` when an LCD is attached.
    /// The two convenience constructors ([`Robot::new`] and
    /// [`Robot::new_with_wifi`]) delegate here with [`NoDisplay`] / [`NoWifi`]
    /// defaults respectively.
    pub fn new_full(
        motors:  M,
        lidar_l: L,
        lidar_r: L,
        input:   I,
        wifi:    W,
        display: D,
    ) -> Self {
        info!("Robot initialised → Idle");
        Self {
            motors,
            lidar_l,
            lidar_r,
            input,
            wifi,
            display,
            state: RobotState::Idle,
            path: PathBuffer::new(),
            record_start_ms: 0,
            play_idx: 0,
            cmd_start_ms: 0,
            avoid_start_ms: 0,
            avoid_side: None,
            halt_log_ms: None,
            last_tl: 0,
            last_tr: 0,
            last_telemetry_ms: 0,
            last_display_state: RobotState::Idle,
            button_hold_start: None,
        }
    }

    // -----------------------------------------------------------------------
    // Public interface
    // -----------------------------------------------------------------------

    /// Drive the FSM forward by one tick.
    ///
    /// Call this from the main loop at the target frequency (≈100 Hz).
    /// `now_ms` — milliseconds elapsed since boot.
    pub fn tick(&mut self, now_ms: u64) {
        // 1. Poll all hardware ports.
        self.lidar_l.poll();
        self.lidar_r.poll();
        self.lidar_l.tick_staleness();
        self.lidar_r.tick_staleness();
        self.input.poll(now_ms);

        // 2. Poll the WiFi/remote port (drains pending commands into buffers).
        self.wifi.poll_network(now_ms);

        // 3. Dispatch to state handler.
        match self.state {
            RobotState::Idle     => self.tick_idle(now_ms),
            RobotState::Record   => self.tick_record(now_ms),
            RobotState::Ready    => self.tick_ready(now_ms),
            RobotState::Play     => self.tick_play(now_ms),
            RobotState::Avoiding => self.tick_avoiding(now_ms),
            RobotState::Halt     => self.tick_halt(now_ms),
            RobotState::Direct   => self.tick_direct(now_ms),
        }

        // 4. Update LCD row 0 when the state changes.
        if self.state != self.last_display_state {
            self.display.print_row(0, self.state.name());
            self.last_display_state = self.state;
        }

        // 5. Emit telemetry and update LCD row 1 at the configured interval.
        if now_ms.saturating_sub(self.last_telemetry_ms) >= TELEMETRY_INTERVAL_MS {
            let frame = TelemetryFrame {
                state_name: self.state.name(),
                lidar_l_cm: self.lidar_l.distance_cm(),
                lidar_r_cm: self.lidar_r.distance_cm(),
                throttle_l: self.last_tl,
                throttle_r: self.last_tr,
                uptime_ms: now_ms,
            };
            self.wifi.send(&frame);
            self.last_telemetry_ms = now_ms;

            // Row 1: in DIRECT mode show live throttle so the operator can
            // see what the joystick is commanding; otherwise show LIDAR
            // readings in a fixed-width format "Lxxx Rxxx cm".
            // Uses heapless::String so no heap allocation is required.
            use core::fmt::Write as _;
            let mut row1: heapless::String<16> = heapless::String::new();
            if self.state == RobotState::Direct {
                // {:+4} always occupies exactly 4 chars (sign + up to 3 digits
                // for i8 range −100…+100), giving "L{4} R{4}     " = 16 chars.
                let _ = write!(row1, "L{:+4} R{:+4}     ", self.last_tl, self.last_tr);
            } else {
                match (frame.lidar_l_cm, frame.lidar_r_cm) {
                    (Some(l), Some(r)) => { let _ = write!(row1, "L{:>3} R{:>3} cm", l, r); }
                    (Some(l), None)    => { let _ = write!(row1, "L{:>3} R--- cm", l); }
                    (None,    Some(r)) => { let _ = write!(row1, "L--- R{:>3} cm", r); }
                    (None,    None)    => { let _ = write!(row1, "L--- R--- cm   "); }
                }
            }
            self.display.print_row(1, row1.as_str());
        }
    }

    /// Return the current FSM state.
    ///
    /// Primarily useful for integration tests and telemetry dashboards;
    /// production code should not branch on state externally.
    pub fn state(&self) -> RobotState {
        self.state
    }

    // -----------------------------------------------------------------------
    // State handlers
    // -----------------------------------------------------------------------

    fn tick_idle(&mut self, now_ms: u64) {
        trace!("IDLE");

        // WiFi button: immediate RECORD (remote control — no long-press UX needed).
        if self.wifi.poll_button() {
            info!("IDLE → RECORD (WiFi)");
            self.path.clear();
            self.record_start_ms = now_ms;
            self.button_hold_start = None;
            self.state = RobotState::Record;
            return;
        }

        // Physical button: record the moment the press edge arrives.
        // A second press while a hold is already in progress is silently
        // dropped (debounce makes this unlikely in practice).
        if self.input.take_button_press() {
            if self.button_hold_start.is_none() {
                self.button_hold_start = Some(now_ms);
            } else {
                trace!("button press ignored — hold already in progress");
            }
        }

        // Decide on button release: long press → DIRECT, short press → RECORD.
        if let Some(hold_start) = self.button_hold_start {
            if !self.input.is_button_held() {
                let held_ms = now_ms.saturating_sub(hold_start);
                self.button_hold_start = None;
                if held_ms >= LONG_PRESS_MS {
                    info!("IDLE → DIRECT (long press {}ms)", held_ms);
                    self.state = RobotState::Direct;
                } else {
                    info!("IDLE → RECORD (short press {}ms)", held_ms);
                    self.path.clear();
                    self.record_start_ms = now_ms;
                    self.state = RobotState::Record;
                }
            }
        }
    }

    fn tick_record(&mut self, now_ms: u64) {
        trace!("RECORD path_len={}", self.path.len());

        if self.input.take_button_press() || self.wifi.poll_button() {
            info!("RECORD → READY  ({} commands)", self.path.len());
            self.do_coast();
            self.state = RobotState::Ready;
            return;
        }

        // Remote throttle takes priority over the physical joystick.
        let (tl, tr) = self
            .wifi
            .poll_throttle()
            .unwrap_or_else(|| (self.input.throttle_left(), self.input.throttle_right()));

        // Sample at PATH_CMD_INTERVAL_MS resolution.
        if now_ms.saturating_sub(self.record_start_ms) >= PATH_CMD_INTERVAL_MS {
            let elapsed = (now_ms - self.record_start_ms).min(u16::MAX as u64) as u16;
            let cmd = PathCommand {
                throttle_l: tl,
                throttle_r: tr,
                duration_ms: elapsed,
            };

            if self.path.push(cmd).is_err() {
                error!("Path buffer full → HALT");
                self.do_coast();
                self.state = RobotState::Halt;
                return;
            }
            debug!(
                "REC cmd #{}: L={:+} R={:+} dur={}ms",
                self.path.len(),
                tl,
                tr,
                elapsed
            );
            self.record_start_ms = now_ms;
        }

        // Drive motors live during recording so the operator sees the result.
        self.do_drive(tl, tr);
    }

    fn tick_ready(&mut self, now_ms: u64) {
        trace!("READY");
        if self.input.take_button_press() || self.wifi.poll_button() {
            if self.path.is_empty() {
                warn!("READY → IDLE (empty path)");
                self.state = RobotState::Idle;
            } else {
                info!("READY → PLAY");
                self.play_idx = 0;
                self.cmd_start_ms = now_ms;
                self.state = RobotState::Play;
            }
        }
    }

    fn tick_play(&mut self, now_ms: u64) {
        // Proximity log (once per tick, only while moving).
        let dl = self.lidar_l.distance_cm();
        let dr = self.lidar_r.distance_cm();
        Self::log_proximity(dl, dr);

        // Obstacle detection: `map_or(false, …)` intentionally treats a
        // stale / None reading as "not blocked" — a missing sensor does NOT
        // trigger avoidance.  Rationale: a disconnected sensor would cause
        // perpetual AVOIDING; the risk of a brief stale window going
        // undetected is lower than the risk of a phantom avoidance loop.
        // (The inverse policy — stale = blocked — applies in Phase 3 of
        // tick_avoiding, where the robot waits until both sensors explicitly
        // confirm the path is clear before resuming.)
        let l_blocked = dl.map_or(false, |d| d < OBSTACLE_CM);
        let r_blocked = dr.map_or(false, |d| d < OBSTACLE_CM);

        if l_blocked || r_blocked {
            let side = match (l_blocked, r_blocked) {
                (true, true)  => ObstacleSide::Both,
                (true, false) => ObstacleSide::Left,
                (false, true) => ObstacleSide::Right,
                _             => unreachable!(),
            };
            warn!("PLAY → AVOIDING  side={:?}  L={:?}cm  R={:?}cm", side, dl, dr);
            self.avoid_side = Some(side);
            self.avoid_start_ms = now_ms;
            self.do_coast();
            self.state = RobotState::Avoiding;
            return;
        }

        // Path-complete check comes before the drive so we coast on the last tick.
        if self.play_idx >= self.path.len() {
            info!("PLAY → HALT (path complete)");
            self.do_coast();
            self.state = RobotState::Halt;
            return;
        }

        // Drive the *current* command every tick so the motors are always
        // receiving the correct duty.  This also ensures the first command
        // (play_idx == 0) is applied immediately on the first tick in PLAY.
        let (tl, tr, dur) = {
            let cmd = &self.path[self.play_idx];
            (cmd.throttle_l, cmd.throttle_r, cmd.duration_ms)
        };
        self.do_drive(tl, tr);

        // Advance to the next command when the current segment has elapsed.
        if now_ms.saturating_sub(self.cmd_start_ms) >= dur as u64 {
            debug!(
                "PLAY cmd {}/{} done (dur={}ms), advancing",
                self.play_idx,
                self.path.len(),
                dur
            );
            self.play_idx += 1;
            self.cmd_start_ms = now_ms;
        }
    }

    fn tick_avoiding(&mut self, now_ms: u64) {
        let elapsed = now_ms.saturating_sub(self.avoid_start_ms);

        // Phase 1 — reverse for AVOID_BACK_MS.
        if elapsed < AVOID_BACK_MS {
            trace!("AVOIDING phase=back elapsed={}ms", elapsed);
            self.do_drive(-50, -50);
            return;
        }

        // Phase 2 — turn for AVOID_TURN_MS.
        // To avoid an obstacle, the robot must turn *away* from it:
        //   Left obstacle  → turn RIGHT  = L forward, R backward = drive( 50, -50)
        //   Right obstacle → turn LEFT   = L backward, R forward = drive(-50,  50)
        //   Both sides     → turn right by convention (arbitrary choice)
        if elapsed < AVOID_BACK_MS + AVOID_TURN_MS {
            trace!("AVOIDING phase=turn elapsed={}ms", elapsed);
            match self.avoid_side {
                Some(ObstacleSide::Left) | Some(ObstacleSide::Both) => {
                    self.do_drive(50, -50); // turn right (away from left)
                }
                Some(ObstacleSide::Right) => {
                    self.do_drive(-50, 50); // turn left (away from right)
                }
                None => self.do_drive(50, -50), // fallback: turn right
            }
            return;
        }

        // Phase 3 — stop motors, then check if path is clear.
        // DRV8833 is set-and-forget: without an explicit coast here the motors
        // continue at Phase 2's last duty cycle indefinitely.
        self.do_coast();

        let dl = self.lidar_l.distance_cm();
        let dr = self.lidar_r.distance_cm();
        let l_clear = dl.map_or(false, |d| d >= CLEAR_CM);
        let r_clear = dr.map_or(false, |d| d >= CLEAR_CM);

        if l_clear && r_clear {
            info!(
                "AVOIDING → PLAY  (path clear  L={:?}cm  R={:?}cm  elapsed={}ms)",
                dl, dr, elapsed
            );
            self.avoid_side = None;
            self.cmd_start_ms = now_ms; // reset segment timer

            // Immediately re-drive the current path command so the robot
            // resumes motion without waiting for the next tick.
            if self.play_idx < self.path.len() {
                let (tl, tr) = {
                    let cmd = &self.path[self.play_idx];
                    (cmd.throttle_l, cmd.throttle_r)
                };
                self.do_drive(tl, tr);
            }

            self.state = RobotState::Play;
            return;
        }

        // Timeout — give up.
        if elapsed >= AVOID_TIMEOUT_MS {
            error!(
                "AVOIDING → HALT  (timeout {}ms  L={:?}cm  R={:?}cm)",
                elapsed, dl, dr
            );
            self.do_coast();
            self.state = RobotState::Halt;
        }
    }

    fn tick_halt(&mut self, now_ms: u64) {
        let should_log = match self.halt_log_ms {
            None       => true,
            Some(last) => now_ms.saturating_sub(last) >= HALT_LOG_INTERVAL_MS,
        };
        if should_log {
            warn!("HALT — power cycle to reset");
            self.halt_log_ms = Some(now_ms);
        }
        self.do_coast();
    }

    fn tick_direct(&mut self, _now_ms: u64) {
        // Any button press (short or long) exits back to IDLE and coasts motors.
        // Note: the WiFi button has no effect in DIRECT mode by design.
        if self.input.take_button_press() {
            info!("DIRECT → IDLE");
            self.do_coast();
            self.state = RobotState::Idle;
            return;
        }

        // Pass joystick throttle directly to the motors — no LIDAR, no path logic.
        let tl = self.input.throttle_left();
        let tr = self.input.throttle_right();
        trace!("DIRECT L={:+} R={:+}", tl, tr);
        self.do_drive(tl, tr);
    }

    // -----------------------------------------------------------------------
    // Motor helpers — track last throttle for telemetry
    // -----------------------------------------------------------------------

    /// Drive both motors and record the throttle values for telemetry.
    fn do_drive(&mut self, tl: i8, tr: i8) {
        self.motors.drive(tl, tr);
        self.last_tl = tl;
        self.last_tr = tr;
    }

    /// Coast both motors and zero the tracked throttle.
    fn do_coast(&mut self) {
        self.motors.coast();
        self.last_tl = 0;
        self.last_tr = 0;
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Emit a WARN trace when either sensor enters the WARN_CM zone.
    fn log_proximity(dl: Option<u16>, dr: Option<u16>) {
        let warn_l = dl.map_or(false, |d| d < WARN_CM);
        let warn_r = dr.map_or(false, |d| d < WARN_CM);
        if warn_l || warn_r {
            warn!("proximity  L={:?}cm  R={:?}cm", dl, dr);
        }
    }
}

// =============================================================================
// Unit tests — FSM integration with mock port implementations
// =============================================================================
//
// These tests run on the host (`cargo +stable test --lib --target aarch64-apple-darwin`)
// because the domain layer has zero `esp-hal` dependencies.  Mock adapters
// implement the three port traits using simple in-memory state so the full
// FSM can be exercised without any hardware.
//
// Run with:
//   cargo +stable test --lib --target aarch64-apple-darwin
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AVOID_BACK_MS, AVOID_TIMEOUT_MS, AVOID_TURN_MS, CLEAR_CM, LONG_PRESS_MS, OBSTACLE_CM,
        PATH_CMD_INTERVAL_MS, TELEMETRY_INTERVAL_MS,
    };

    // -------------------------------------------------------------------------
    // Mock port implementations
    // -------------------------------------------------------------------------

    /// Records every `drive()` call and whether `coast()` was ever called.
    #[derive(Default)]
    struct MockMotors {
        drives: Vec<(i8, i8)>,
        coasts: u32,
    }

    impl MockMotors {
        fn last_drive(&self) -> Option<(i8, i8)> {
            self.drives.last().copied()
        }
    }

    impl MotorPort for MockMotors {
        fn drive(&mut self, left: i8, right: i8) {
            self.drives.push((left, right));
        }
        fn coast(&mut self) {
            self.coasts += 1;
        }
    }

    // -------------------------------------------------------------------------

    /// A distance sensor that always returns a fixed value until updated.
    #[derive(Default)]
    struct MockDistance {
        dist: Option<u16>,
    }

    impl MockDistance {
        fn set(&mut self, cm: u16) {
            self.dist = Some(cm);
        }
        fn clear(&mut self) {
            self.dist = None;
        }
    }

    impl DistancePort for MockDistance {
        fn poll(&mut self) {}
        fn distance_cm(&self) -> Option<u16> {
            self.dist
        }
        fn tick_staleness(&mut self) {}
    }

    // -------------------------------------------------------------------------

    /// Input source backed by a queue of button-press events and a fixed throttle.
    #[derive(Default)]
    struct MockInput {
        pending_presses: usize,
        /// Simulates the raw "is button physically held down" GPIO level.
        ///
        /// Defaults to `false` (not held), which makes existing tests that call
        /// `press()` behave as an instantaneous click: `take_button_press()` fires
        /// once and `is_button_held()` immediately returns `false`, so `tick_idle`
        /// transitions to RECORD in the same tick (held_ms = 0 < LONG_PRESS_MS).
        btn_held: bool,
        tl: i8,
        tr: i8,
    }

    impl MockInput {
        /// Enqueue N synthetic button presses.
        fn press_n(&mut self, n: usize) {
            self.pending_presses += n;
        }
        fn press(&mut self) {
            self.press_n(1);
        }
        /// Simulate the button being pressed AND held (sets held state + queues edge).
        fn press_and_hold(&mut self) {
            self.pending_presses += 1;
            self.btn_held = true;
        }
        /// Simulate the button being released.
        fn release(&mut self) {
            self.btn_held = false;
        }
        fn set_throttle(&mut self, left: i8, right: i8) {
            self.tl = left;
            self.tr = right;
        }
    }

    impl InputPort for MockInput {
        fn poll(&mut self, _now_ms: u64) {}
        fn throttle_left(&self) -> i8 {
            self.tl
        }
        fn throttle_right(&self) -> i8 {
            self.tr
        }
        fn take_button_press(&mut self) -> bool {
            if self.pending_presses > 0 {
                self.pending_presses -= 1;
                true
            } else {
                false
            }
        }
        fn is_button_held(&self) -> bool {
            self.btn_held
        }
    }

    // -------------------------------------------------------------------------

    /// WiFi stub — records telemetry frames and sources synthetic remote events.
    #[derive(Default)]
    struct MockWifi {
        /// Queued button presses; `poll_button()` drains one per call.
        button_presses: usize,
        /// Next throttle override; `poll_throttle()` takes it (clears to None).
        pending_throttle: Option<(i8, i8)>,
        /// All frames delivered via `TelemetryPort::send`.
        sent_frames: Vec<TelemetryFrame>,
    }

    impl RemoteControlPort for MockWifi {
        fn poll_network(&mut self, _now_ms: u64) {}
        fn poll_throttle(&mut self) -> Option<(i8, i8)> {
            self.pending_throttle.take()
        }
        fn poll_button(&mut self) -> bool {
            if self.button_presses > 0 {
                self.button_presses -= 1;
                true
            } else {
                false
            }
        }
    }

    impl TelemetryPort for MockWifi {
        fn send(&mut self, frame: &TelemetryFrame) {
            self.sent_frames.push(*frame);
        }
    }

    // -------------------------------------------------------------------------

    /// No-op display for unit tests — silently discards all writes.
    #[derive(Default)]
    struct MockDisplay;

    impl DisplayPort for MockDisplay {
        fn print_row(&mut self, _row: u8, _text: &str) {}
        fn clear(&mut self) {}
    }

    /// Recording display — captures every `print_row` call so tests can
    /// assert exactly what was written to each LCD row.
    #[derive(Default)]
    struct RecordingDisplay {
        row0: Vec<String>,
        row1: Vec<String>,
    }

    impl RecordingDisplay {
        fn last_row1(&self) -> &str {
            self.row1.last().map(String::as_str).unwrap_or("")
        }
    }

    impl DisplayPort for RecordingDisplay {
        fn print_row(&mut self, row: u8, text: &str) {
            match row {
                0 => self.row0.push(text.to_string()),
                1 => self.row1.push(text.to_string()),
                _ => {}
            }
        }
        fn clear(&mut self) {
            self.row0.clear();
            self.row1.clear();
        }
    }

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    /// Construct a fresh Robot with clear sensors and zero throttle.
    fn make_robot() -> Robot<MockMotors, MockDistance, MockInput> {
        Robot::new(
            MockMotors::default(),
            MockDistance::default(),
            MockDistance::default(),
            MockInput::default(),
        )
    }

    /// Tick the robot `n` times advancing time by 1 ms per tick.
    fn tick_n(robot: &mut Robot<MockMotors, MockDistance, MockInput>, start_ms: u64, n: u64) {
        for i in 0..n {
            robot.tick(start_ms + i);
        }
    }

    /// Construct a fresh Robot with a `MockWifi` attached.
    fn make_robot_with_wifi() -> Robot<MockMotors, MockDistance, MockInput, MockWifi> {
        Robot::new_with_wifi(
            MockMotors::default(),
            MockDistance::default(),
            MockDistance::default(),
            MockInput::default(),
            MockWifi::default(),
        )
    }

    /// Construct a fresh Robot with a `RecordingDisplay` attached.
    fn make_robot_with_display() -> Robot<MockMotors, MockDistance, MockInput, NoWifi, RecordingDisplay> {
        Robot::new_full(
            MockMotors::default(),
            MockDistance::default(),
            MockDistance::default(),
            MockInput::default(),
            NoWifi,
            RecordingDisplay::default(),
        )
    }

    /// Build a Robot already in `Avoiding` state (left sensor triggered).
    ///
    /// `avoid_start_ms` will be `PATH_CMD_INTERVAL_MS + 3`.
    /// `avoid_side` will be `Some(ObstacleSide::Left)`.
    fn robot_in_avoiding() -> Robot<MockMotors, MockDistance, MockInput> {
        let mut r = robot_in_play();
        let t_avoid = PATH_CMD_INTERVAL_MS + 3;
        r.lidar_l.set(OBSTACLE_CM - 1);
        r.tick(t_avoid);
        assert_eq!(r.state, RobotState::Avoiding);
        assert_eq!(r.avoid_side, Some(ObstacleSide::Left));
        r
    }

    // -------------------------------------------------------------------------
    // State-transition tests
    // -------------------------------------------------------------------------

    #[test]
    fn idle_button_transitions_to_record() {
        let mut r = make_robot();
        r.input.press();
        r.tick(0);
        assert_eq!(r.state, RobotState::Record);
    }

    #[test]
    fn record_button_transitions_to_ready() {
        let mut r = make_robot();
        // Button 1: Idle → Record
        r.input.press();
        r.tick(0);
        assert_eq!(r.state, RobotState::Record);

        // Button 2: Record → Ready
        r.input.press();
        r.tick(1);
        assert_eq!(r.state, RobotState::Ready);
    }

    #[test]
    fn ready_with_empty_path_returns_to_idle() {
        let mut r = make_robot();
        // Idle → Record → Ready (no motion recorded)
        r.input.press();
        r.tick(0);
        r.input.press();
        r.tick(1);
        assert_eq!(r.state, RobotState::Ready);
        assert!(r.path.is_empty(), "path should be empty (no ticks during recording)");

        // Button 3 on empty path → back to Idle
        r.input.press();
        r.tick(2);
        assert_eq!(r.state, RobotState::Idle);
    }

    #[test]
    fn ready_button_with_path_transitions_to_play() {
        let mut r = make_robot();
        r.input.set_throttle(50, 50);

        // Idle → Record
        r.input.press();
        r.tick(0);

        // Tick enough times to record at least one command.
        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t);
        }
        // Record → Ready
        r.input.press();
        r.tick(PATH_CMD_INTERVAL_MS + 1);
        assert_eq!(r.state, RobotState::Ready);
        assert!(!r.path.is_empty(), "path must have at least one command");

        // Ready → Play
        r.input.press();
        r.tick(PATH_CMD_INTERVAL_MS + 2);
        assert_eq!(r.state, RobotState::Play);
    }

    // -------------------------------------------------------------------------
    // PLAY state tests
    // -------------------------------------------------------------------------

    #[test]
    fn play_drives_first_command_immediately() {
        // Regression test for BUG #1: the first path command must be driven on
        // the very first tick where tick_play runs (immediately after READY→PLAY),
        // not after its duration has elapsed.
        let mut r = make_robot();
        r.input.set_throttle(70, 60);

        // Idle → Record → (record one command) → Ready → Play
        r.input.press();
        r.tick(0);
        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t);
        }
        r.input.press();
        let t_ready = PATH_CMD_INTERVAL_MS + 1;
        r.tick(t_ready);
        r.input.press();
        let t_play = t_ready + 1;

        // The transition tick (tick_ready) changes state to Play but does NOT
        // call tick_play yet.  Snapshot drives count so we can check the NEXT tick.
        r.tick(t_play);
        assert_eq!(r.state, RobotState::Play);
        let drives_at_transition = r.motors.drives.len();

        // First actual tick_play call — must drive path[0] immediately.
        r.tick(t_play + 1);
        let new_drives = &r.motors.drives[drives_at_transition..];
        assert!(
            !new_drives.is_empty(),
            "tick_play must call drive() on its very first invocation"
        );
        let &(l, rt) = new_drives.first().unwrap();
        assert_eq!(l, 70, "left throttle of first command must match recorded value");
        assert_eq!(rt, 60, "right throttle of first command must match recorded value");
    }

    #[test]
    fn play_advances_commands_by_duration() {
        let mut r = make_robot();

        // Record two commands with different throttle values.
        r.input.set_throttle(30, 30);
        r.input.press(); // Idle → Record
        r.tick(0);

        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t); // first command recorded at t=PATH_CMD_INTERVAL_MS
        }

        r.input.set_throttle(80, -80); // change throttle for second command
        for t in (PATH_CMD_INTERVAL_MS + 1)..=(PATH_CMD_INTERVAL_MS * 2) {
            r.tick(t); // second command recorded
        }

        r.input.press(); // Record → Ready
        let t0 = PATH_CMD_INTERVAL_MS * 2 + 1;
        r.tick(t0);
        r.input.press(); // Ready → Play (transition tick: tick_ready runs)
        let t1 = t0 + 1;
        r.tick(t1);
        assert_eq!(r.state, RobotState::Play);

        // One tick to invoke tick_play for the first time.
        let t2 = t1 + 1;
        r.tick(t2);
        let (l, _) = r.motors.last_drive().unwrap();
        assert_eq!(l, 30, "first command should be active on first tick_play");

        // Advance past first command's duration.
        let first_duration = r.path[0].duration_ms as u64;
        for t in (t2 + 1)..=(t2 + first_duration + 1) {
            r.tick(t);
        }

        // Now the second command should be active.
        let (l2, _) = r.motors.last_drive().unwrap();
        assert_eq!(l2, 80, "second command must be active after first expires");
    }

    #[test]
    fn play_halts_when_path_complete() {
        let mut r = make_robot();
        r.input.set_throttle(20, 20);

        r.input.press();
        r.tick(0);
        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t);
        }
        r.input.press();
        let t0 = PATH_CMD_INTERVAL_MS + 1;
        r.tick(t0);
        r.input.press();
        let t1 = t0 + 1;
        r.tick(t1);

        assert_eq!(r.state, RobotState::Play);
        let total_duration: u64 = r.path.iter().map(|c| c.duration_ms as u64).sum();

        // Run well past total path duration.
        tick_n(&mut r, t1 + 1, total_duration + 50);
        assert_eq!(r.state, RobotState::Halt, "robot must halt after all commands replay");
    }

    #[test]
    fn play_obstacle_triggers_avoiding() {
        let mut r = make_robot();
        r.input.set_throttle(50, 50);

        r.input.press();
        r.tick(0);
        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t);
        }
        r.input.press();
        let t0 = PATH_CMD_INTERVAL_MS + 1;
        r.tick(t0);
        r.input.press();
        let t1 = t0 + 1;
        r.tick(t1);
        assert_eq!(r.state, RobotState::Play);

        // Simulate right sensor detecting obstacle.
        r.lidar_r.set(OBSTACLE_CM - 1);
        r.tick(t1 + 1);
        assert_eq!(r.state, RobotState::Avoiding);
        assert_eq!(r.avoid_side, Some(ObstacleSide::Right));
    }

    #[test]
    fn play_stale_sensor_does_not_trigger_avoiding() {
        // A None reading (sensor stale or disconnected) must NOT trigger avoidance;
        // design decision: treat unknown distance as safe to avoid false positives
        // from sensor disconnects.
        let mut r = make_robot();
        r.input.set_throttle(50, 50);

        r.input.press();
        r.tick(0);
        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t);
        }
        r.input.press();
        let t0 = PATH_CMD_INTERVAL_MS + 1;
        r.tick(t0);
        r.input.press();
        let t1 = t0 + 1;
        r.tick(t1);
        assert_eq!(r.state, RobotState::Play);

        // No distance set → sensors return None.
        r.tick(t1 + 1);
        assert_eq!(r.state, RobotState::Play, "stale sensor must not trigger avoiding");
    }

    // -------------------------------------------------------------------------
    // AVOIDING state tests
    // -------------------------------------------------------------------------

    /// Helper: build a Robot already in Play state with one long path command.
    fn robot_in_play() -> Robot<MockMotors, MockDistance, MockInput> {
        let mut r = make_robot();
        r.input.set_throttle(50, 50);
        r.input.press();
        r.tick(0);
        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t);
        }
        r.input.press();
        let t0 = PATH_CMD_INTERVAL_MS + 1;
        r.tick(t0);
        r.input.press();
        r.tick(t0 + 1);
        // Stretch the recorded command so it doesn't expire during the test.
        if let Some(cmd) = r.path.first_mut() {
            cmd.duration_ms = 60_000; // 60 s — will not expire during test
        }
        assert_eq!(r.state, RobotState::Play);
        r
    }

    #[test]
    fn avoiding_back_phase_drives_backward() {
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;

        r.lidar_l.set(OBSTACLE_CM - 1);
        r.tick(t0); // triggers AVOIDING
        assert_eq!(r.state, RobotState::Avoiding);

        let avoid_start = r.avoid_start_ms;
        // During back phase, drive(-50,-50) must be applied.
        r.tick(avoid_start + 1);
        assert_eq!(
            r.motors.last_drive(),
            Some((-50, -50)),
            "back phase must drive both motors in reverse"
        );
    }

    #[test]
    fn avoiding_left_obstacle_turns_right() {
        // Left obstacle → turn right = drive(+50, -50) (L fwd, R rev).
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;

        r.lidar_l.set(OBSTACLE_CM - 1);
        r.tick(t0);
        assert_eq!(r.state, RobotState::Avoiding);

        let avoid_start = r.avoid_start_ms;
        // Skip past back phase into turn phase.
        let turn_start = avoid_start + AVOID_BACK_MS + 1;
        r.tick(turn_start);
        assert_eq!(
            r.motors.last_drive(),
            Some((50, -50)),
            "left obstacle: turn phase must drive left-fwd/right-rev (turn right)"
        );
    }

    #[test]
    fn avoiding_right_obstacle_turns_left() {
        // Right obstacle → turn left = drive(-50, +50) (L rev, R fwd).
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;

        r.lidar_r.set(OBSTACLE_CM - 1);
        r.tick(t0);
        assert_eq!(r.state, RobotState::Avoiding);

        let avoid_start = r.avoid_start_ms;
        let turn_start = avoid_start + AVOID_BACK_MS + 1;
        r.tick(turn_start);
        assert_eq!(
            r.motors.last_drive(),
            Some((-50, 50)),
            "right obstacle: turn phase must drive left-rev/right-fwd (turn left)"
        );
    }

    #[test]
    fn avoiding_both_sides_turns_right_by_convention() {
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;

        r.lidar_l.set(OBSTACLE_CM - 1);
        r.lidar_r.set(OBSTACLE_CM - 1);
        r.tick(t0);
        assert_eq!(r.state, RobotState::Avoiding);
        assert_eq!(r.avoid_side, Some(ObstacleSide::Both));

        let avoid_start = r.avoid_start_ms;
        let turn_start = avoid_start + AVOID_BACK_MS + 1;
        r.tick(turn_start);
        assert_eq!(
            r.motors.last_drive(),
            Some((50, -50)),
            "both sides: convention is to turn right"
        );
    }

    #[test]
    fn avoiding_resumes_play_when_clear() {
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;

        r.lidar_l.set(OBSTACLE_CM - 1);
        r.tick(t0); // PLAY → AVOIDING
        assert_eq!(r.state, RobotState::Avoiding);

        let avoid_start = r.avoid_start_ms;
        // Advance past back+turn phases and clear the obstacle.
        let post_turn = avoid_start + AVOID_BACK_MS + AVOID_TURN_MS + 1;
        r.lidar_l.clear();
        r.lidar_l.set(200); // well above CLEAR_CM (100)
        r.lidar_r.set(200);
        r.tick(post_turn);

        assert_eq!(r.state, RobotState::Play, "robot must resume Play when path is clear");
    }

    #[test]
    fn avoiding_resume_drives_current_command() {
        // Regression test for BUG #2: after AVOIDING → PLAY, the current path
        // command must be driven immediately, not after the next tick's duration check.
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;
        let expected_throttle = (r.path[0].throttle_l, r.path[0].throttle_r);

        r.lidar_r.set(OBSTACLE_CM - 1);
        r.tick(t0);
        assert_eq!(r.state, RobotState::Avoiding);

        let avoid_start = r.avoid_start_ms;
        let post_turn = avoid_start + AVOID_BACK_MS + AVOID_TURN_MS + 1;
        r.lidar_r.clear();
        r.lidar_r.set(200);
        r.lidar_l.set(200);
        r.tick(post_turn); // AVOIDING → PLAY, should immediately re-drive

        assert_eq!(r.state, RobotState::Play);
        assert_eq!(
            r.motors.last_drive(),
            Some(expected_throttle),
            "path command must be re-applied immediately on AVOIDING→PLAY transition"
        );
    }

    #[test]
    fn avoiding_timeout_triggers_halt() {
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;

        r.lidar_l.set(OBSTACLE_CM - 1);
        r.tick(t0);
        assert_eq!(r.state, RobotState::Avoiding);

        let avoid_start = r.avoid_start_ms;
        // Keep obstacle present past the timeout.
        r.lidar_l.set(OBSTACLE_CM - 1);
        r.lidar_r.set(OBSTACLE_CM - 1);
        r.tick(avoid_start + AVOID_TIMEOUT_MS + 1);

        assert_eq!(r.state, RobotState::Halt, "robot must halt after avoidance timeout");
        assert!(r.motors.coasts > 0, "coast must be called on timeout");
    }

    // -------------------------------------------------------------------------
    // HALT state tests
    // -------------------------------------------------------------------------

    #[test]
    fn halt_coasts_motors() {
        // Use robot_in_play() which gives us a properly set-up Play state,
        // then trigger HALT via avoidance timeout.
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;

        r.lidar_l.set(1); // obstacle on left
        r.tick(t0); // Play → Avoiding
        assert_eq!(r.state, RobotState::Avoiding);

        // Keep both sensors blocked until timeout fires.
        r.lidar_r.set(1);
        let avoid_start = r.avoid_start_ms;
        r.tick(avoid_start + AVOID_TIMEOUT_MS + 1); // Avoiding → Halt

        assert_eq!(r.state, RobotState::Halt, "should be in Halt after timeout");
        let coasts_at_halt = r.motors.coasts;
        assert!(coasts_at_halt > 0, "coast must be called on Halt entry");

        // Additional ticks in Halt must keep calling coast().
        r.tick(avoid_start + AVOID_TIMEOUT_MS + 2);
        assert!(
            r.motors.coasts > coasts_at_halt,
            "halt must call coast() every tick"
        );
    }

    #[test]
    fn halt_repeats_coast_on_subsequent_ticks() {
        let mut r = make_robot();
        // Get into Halt
        r.input.set_throttle(50, 50);
        r.input.press();
        r.tick(0);
        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t);
        }
        r.input.press();
        let t0 = PATH_CMD_INTERVAL_MS + 1;
        r.tick(t0);
        r.input.press();
        let t1 = t0 + 1;
        r.tick(t1);
        let total: u64 = r.path.iter().map(|c| c.duration_ms as u64).sum();
        tick_n(&mut r, t1 + 1, total + 50);
        assert_eq!(r.state, RobotState::Halt);
        let coasts_before = r.motors.coasts;
        r.tick(t1 + total + 100);
        assert!(
            r.motors.coasts > coasts_before,
            "halt must keep calling coast() on every tick"
        );
    }

    // -------------------------------------------------------------------------
    // Buffer overflow → HALT
    // -------------------------------------------------------------------------

    #[test]
    fn record_buffer_overflow_triggers_halt() {
        use crate::domain::path::PATH_CAPACITY;

        let mut r = make_robot();
        r.input.set_throttle(10, 10);
        r.input.press(); // Idle → Record
        r.tick(0);

        // Tick at 1 ms intervals; each PATH_CMD_INTERVAL_MS ticks pushes one entry.
        // We need PATH_CAPACITY + 1 pushes to overflow.
        let needed_ticks = (PATH_CAPACITY as u64 + 2) * PATH_CMD_INTERVAL_MS;
        for t in 1..=needed_ticks {
            r.tick(t);
            if r.state == RobotState::Halt {
                break;
            }
        }
        assert_eq!(
            r.state,
            RobotState::Halt,
            "buffer overflow must transition to Halt"
        );
    }

    // =========================================================================
    // Additional coverage: edge-cases, boundary conditions, WiFi paths
    // =========================================================================

    // -------------------------------------------------------------------------
    // IDLE — additional coverage
    // -------------------------------------------------------------------------

    #[test]
    fn idle_no_input_stays_idle() {
        let mut r = make_robot();
        r.tick(0);
        r.tick(1);
        r.tick(100);
        assert_eq!(r.state, RobotState::Idle, "no button → must stay Idle indefinitely");
    }

    #[test]
    fn idle_wifi_button_transitions_to_record() {
        let mut r = make_robot_with_wifi();
        r.wifi.button_presses = 1;
        r.tick(0);
        assert_eq!(r.state, RobotState::Record, "WiFi button in Idle must trigger Idle→Record");
    }

    // -------------------------------------------------------------------------
    // RECORD — additional coverage
    // -------------------------------------------------------------------------

    #[test]
    fn record_joystick_drives_motors_live() {
        let mut r = make_robot();
        r.input.set_throttle(40, -40);
        r.input.press(); // Idle → Record at t=0
        r.tick(0);
        r.tick(1); // tick_record drives motors from joystick
        assert_eq!(
            r.motors.last_drive(),
            Some((40, -40)),
            "joystick values must be applied to motors every tick during RECORD"
        );
    }

    #[test]
    fn record_remote_throttle_overrides_joystick() {
        let mut r = make_robot_with_wifi();
        r.input.set_throttle(20, 20); // joystick says (20, 20)
        r.wifi.button_presses = 1; // trigger Idle → Record
        r.tick(0);
        assert_eq!(r.state, RobotState::Record);

        // Remote throttle takes priority over physical joystick.
        r.wifi.pending_throttle = Some((70, -70));
        r.tick(1);
        assert_eq!(
            r.motors.last_drive(),
            Some((70, -70)),
            "remote throttle must override joystick in RECORD state"
        );
    }

    #[test]
    fn record_remote_button_transitions_to_ready() {
        let mut r = make_robot_with_wifi();
        r.wifi.button_presses = 1;
        r.tick(0); // Idle → Record
        assert_eq!(r.state, RobotState::Record);

        r.wifi.button_presses = 1;
        r.tick(1); // Record → Ready
        assert_eq!(r.state, RobotState::Ready, "WiFi button in RECORD must trigger Record→Ready");
    }

    #[test]
    fn record_coasts_motors_on_transition_to_ready() {
        let mut r = make_robot();
        r.input.set_throttle(50, 50);
        r.input.press();
        r.tick(0); // Idle → Record
        r.tick(1); // drives motors (50, 50)
        let coasts_before = r.motors.coasts;

        r.input.press();
        r.tick(2); // Record → Ready: do_coast() called
        assert!(
            r.motors.coasts > coasts_before,
            "do_coast() must be called when transitioning Record → Ready"
        );
    }

    #[test]
    fn record_duration_ms_matches_elapsed_interval() {
        let mut r = make_robot();
        r.input.set_throttle(10, 10);
        r.input.press();
        r.tick(0); // Idle → Record, record_start_ms = 0

        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t);
        }
        assert_eq!(r.path.len(), 1);
        assert_eq!(
            r.path[0].duration_ms,
            PATH_CMD_INTERVAL_MS as u16,
            "duration_ms of first recorded command must equal PATH_CMD_INTERVAL_MS"
        );
    }

    #[test]
    fn record_builds_multiple_commands() {
        let mut r = make_robot();
        r.input.set_throttle(30, 30);
        r.input.press();
        r.tick(0);
        for t in 1..=(PATH_CMD_INTERVAL_MS * 3) {
            r.tick(t);
        }
        assert_eq!(
            r.path.len(),
            3,
            "three sampling intervals must produce exactly three path commands"
        );
    }

    // -------------------------------------------------------------------------
    // READY — additional coverage
    // -------------------------------------------------------------------------

    #[test]
    fn ready_stays_in_ready_without_input() {
        let mut r = make_robot();
        r.input.press();
        r.tick(0); // → Record
        r.input.press();
        r.tick(1); // → Ready
        r.tick(2);
        r.tick(10);
        assert_eq!(r.state, RobotState::Ready, "no button in Ready → must stay Ready");
    }

    #[test]
    fn ready_wifi_button_transitions_to_play() {
        let mut r = make_robot_with_wifi();
        r.input.set_throttle(50, 50);

        r.wifi.button_presses = 1;
        r.tick(0); // → Record
        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t);
        }
        r.wifi.button_presses = 1;
        r.tick(PATH_CMD_INTERVAL_MS + 1); // → Ready
        assert!(!r.path.is_empty());

        r.wifi.button_presses = 1;
        r.tick(PATH_CMD_INTERVAL_MS + 2); // → Play
        assert_eq!(r.state, RobotState::Play, "WiFi button in READY must trigger Ready→Play");
    }

    // -------------------------------------------------------------------------
    // PLAY — additional coverage
    // -------------------------------------------------------------------------

    #[test]
    fn play_both_sensors_blocked_triggers_avoiding_both() {
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;
        r.lidar_l.set(OBSTACLE_CM - 1);
        r.lidar_r.set(OBSTACLE_CM - 1);
        r.tick(t0);
        assert_eq!(r.state, RobotState::Avoiding);
        assert_eq!(
            r.avoid_side,
            Some(ObstacleSide::Both),
            "both sensors blocked must set ObstacleSide::Both"
        );
    }

    #[test]
    fn play_obstacle_at_exact_threshold_is_safe() {
        // d == OBSTACLE_CM must NOT trigger avoiding (condition is strictly < ).
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;
        r.lidar_l.set(OBSTACLE_CM); // exactly at threshold, NOT below
        r.tick(t0);
        assert_eq!(
            r.state,
            RobotState::Play,
            "distance == OBSTACLE_CM must NOT trigger Avoiding (strictly < required)"
        );
    }

    #[test]
    fn play_motor_driven_every_tick() {
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;
        let before = r.motors.drives.len();
        r.tick(t0);
        r.tick(t0 + 1);
        r.tick(t0 + 2);
        assert_eq!(
            r.motors.drives.len(),
            before + 3,
            "tick_play must call drive() on every single tick"
        );
    }

    // -------------------------------------------------------------------------
    // AVOIDING — additional coverage
    // -------------------------------------------------------------------------

    #[test]
    fn avoiding_back_phase_at_exact_boundary_still_backing() {
        // elapsed == AVOID_BACK_MS - 1: still in back phase (strictly < AVOID_BACK_MS).
        let mut r = robot_in_avoiding();
        let t = r.avoid_start_ms + AVOID_BACK_MS - 1;
        r.tick(t);
        assert_eq!(
            r.motors.last_drive(),
            Some((-50, -50)),
            "one tick before AVOID_BACK_MS boundary must still be in back phase"
        );
    }

    #[test]
    fn avoiding_turn_phase_at_exact_boundary_still_turning() {
        // elapsed == AVOID_BACK_MS + AVOID_TURN_MS - 1: still in turn phase.
        let mut r = robot_in_avoiding(); // avoid_side = Left → turns right (50, -50)
        let t = r.avoid_start_ms + AVOID_BACK_MS + AVOID_TURN_MS - 1;
        r.tick(t);
        assert_eq!(
            r.motors.last_drive(),
            Some((50, -50)),
            "one tick before back+turn boundary must still be in turn phase"
        );
    }

    #[test]
    fn avoiding_none_side_defaults_to_right_turn() {
        // When avoid_side is None the fallback is to turn right: drive(50, -50).
        let mut r = robot_in_avoiding();
        r.avoid_side = None; // override to exercise the None arm
        let t = r.avoid_start_ms + AVOID_BACK_MS + 1; // well into turn phase
        r.tick(t);
        assert_eq!(
            r.motors.last_drive(),
            Some((50, -50)),
            "None avoid_side must fall back to turning right (50, -50)"
        );
    }

    #[test]
    fn avoiding_only_one_sensor_clear_does_not_resume() {
        // Both sensors must report >= CLEAR_CM before resuming; one clear is not enough.
        let mut r = robot_in_play();
        let t0 = PATH_CMD_INTERVAL_MS + 3;
        r.lidar_l.set(OBSTACLE_CM - 1);
        r.lidar_r.set(OBSTACLE_CM - 1);
        r.tick(t0); // PLAY → AVOIDING

        let post_turn = r.avoid_start_ms + AVOID_BACK_MS + AVOID_TURN_MS + 1;
        r.lidar_l.set(CLEAR_CM + 10); // left clear
        r.lidar_r.set(OBSTACLE_CM - 1); // right still blocked
        r.tick(post_turn);
        assert_eq!(
            r.state,
            RobotState::Avoiding,
            "only one sensor clear must NOT resume Play (both must be >= CLEAR_CM)"
        );
    }

    #[test]
    fn avoiding_clear_at_exact_threshold_does_resume() {
        // distance == CLEAR_CM satisfies the >= CLEAR_CM condition → must resume.
        let mut r = robot_in_avoiding();
        let post_turn = r.avoid_start_ms + AVOID_BACK_MS + AVOID_TURN_MS + 1;
        r.lidar_l.set(CLEAR_CM);
        r.lidar_r.set(CLEAR_CM);
        r.tick(post_turn);
        assert_eq!(
            r.state,
            RobotState::Play,
            "distance == CLEAR_CM must satisfy the >= CLEAR_CM resume condition"
        );
    }

    #[test]
    fn avoiding_play_idx_unchanged_after_resume() {
        // play_idx is NOT reset when returning from Avoiding; we resume mid-path.
        let mut r = robot_in_play();
        let original_idx = r.play_idx;
        let t0 = PATH_CMD_INTERVAL_MS + 3;
        r.lidar_r.set(OBSTACLE_CM - 1);
        r.tick(t0); // PLAY → AVOIDING

        let post_turn = r.avoid_start_ms + AVOID_BACK_MS + AVOID_TURN_MS + 1;
        r.lidar_r.set(CLEAR_CM);
        r.lidar_l.set(CLEAR_CM);
        r.tick(post_turn); // AVOIDING → PLAY
        assert_eq!(
            r.play_idx, original_idx,
            "play_idx must be unchanged when resuming from AVOIDING"
        );
    }

    #[test]
    fn avoiding_side_cleared_on_resume() {
        let mut r = robot_in_avoiding();
        assert!(r.avoid_side.is_some());
        let post_turn = r.avoid_start_ms + AVOID_BACK_MS + AVOID_TURN_MS + 1;
        r.lidar_l.set(CLEAR_CM);
        r.lidar_r.set(CLEAR_CM);
        r.tick(post_turn);
        assert_eq!(r.avoid_side, None, "avoid_side must be reset to None on AVOIDING → PLAY");
    }

    #[test]
    /// During Phase 3 (post-turn, sensors not yet clear) the motors must be
    /// coasted every tick.  Without this guardrail the DRV8833 would keep
    /// spinning at the last Phase-2 duty cycle indefinitely.
    fn avoiding_phase3_coasts_motors_while_waiting() {
        let mut r = robot_in_avoiding();
        // Advance to Phase 3 — leave sensors blocked (default is OBSTACLE_CM - 1).
        let phase3_start = r.avoid_start_ms + AVOID_BACK_MS + AVOID_TURN_MS + 1;
        let drives_before = r.motors.drives.len();
        let coasts_before = r.motors.coasts;
        r.tick(phase3_start);
        assert_eq!(r.state, RobotState::Avoiding, "should still be avoiding");
        assert_eq!(
            r.motors.drives.len(),
            drives_before,
            "no new drive commands must be issued in Phase 3"
        );
        assert!(
            r.motors.coasts > coasts_before,
            "motors must be coasted on Phase 3 entry"
        );
        // A second Phase-3 tick must also coast (not drive).
        let coasts_after_first = r.motors.coasts;
        r.tick(phase3_start + 1);
        assert!(
            r.motors.coasts > coasts_after_first,
            "motors must stay coasted on subsequent Phase 3 ticks"
        );
    }

    // -------------------------------------------------------------------------
    // HALT — additional coverage
    // -------------------------------------------------------------------------

    /// Drive the robot through Record → Play → Halt (path complete), returning
    /// the time of the last tick so callers can continue from a known timestamp.
    fn robot_in_halt() -> (Robot<MockMotors, MockDistance, MockInput>, u64) {
        let mut r = make_robot();
        r.input.set_throttle(50, 50);
        r.input.press();
        r.tick(0);
        for t in 1..=PATH_CMD_INTERVAL_MS {
            r.tick(t);
        }
        r.input.press();
        let t0 = PATH_CMD_INTERVAL_MS + 1;
        r.tick(t0);
        r.input.press();
        let t1 = t0 + 1;
        r.tick(t1);
        let total: u64 = r.path.iter().map(|c| c.duration_ms as u64).sum();
        let t_end = t1 + total + 50;
        tick_n(&mut r, t1 + 1, total + 50);
        assert_eq!(r.state, RobotState::Halt);
        (r, t_end)
    }

    #[test]
    fn halt_ignores_button_press() {
        let (mut r, t_end) = robot_in_halt();
        r.input.press();
        r.tick(t_end + 1);
        assert_eq!(
            r.state,
            RobotState::Halt,
            "Halt is a terminal state — button press must be ignored"
        );
    }

    #[test]
    fn halt_zeros_motor_throttle_tracking() {
        let (r, _) = robot_in_halt();
        assert_eq!(r.last_tl, 0, "do_coast() must zero last_tl on Halt entry");
        assert_eq!(r.last_tr, 0, "do_coast() must zero last_tr on Halt entry");
    }

    // -------------------------------------------------------------------------
    // TELEMETRY — interval and frame-content tests
    // -------------------------------------------------------------------------

    #[test]
    fn telemetry_not_sent_before_interval_elapses() {
        let mut r = make_robot_with_wifi();
        for t in 0..TELEMETRY_INTERVAL_MS {
            r.tick(t);
        }
        assert!(
            r.wifi.sent_frames.is_empty(),
            "no telemetry frame must be emitted before TELEMETRY_INTERVAL_MS elapses"
        );
    }

    #[test]
    fn telemetry_sent_at_interval() {
        let mut r = make_robot_with_wifi();
        for t in 0..=TELEMETRY_INTERVAL_MS {
            r.tick(t);
        }
        assert_eq!(
            r.wifi.sent_frames.len(),
            1,
            "exactly one telemetry frame must be emitted at t == TELEMETRY_INTERVAL_MS"
        );
    }

    #[test]
    fn telemetry_resent_after_second_interval() {
        let mut r = make_robot_with_wifi();
        for t in 0..=(TELEMETRY_INTERVAL_MS * 2) {
            r.tick(t);
        }
        assert_eq!(
            r.wifi.sent_frames.len(),
            2,
            "two telemetry frames must be emitted after two full intervals"
        );
    }

    #[test]
    fn telemetry_frame_contains_correct_state_and_throttle() {
        let mut r = make_robot_with_wifi();
        // Robot starts Idle with zero throttle.
        for t in 0..=TELEMETRY_INTERVAL_MS {
            r.tick(t);
        }
        let frame = r.wifi.sent_frames.last().expect("a frame must have been sent");
        assert_eq!(frame.state_name, "IDLE");
        assert_eq!(frame.throttle_l, 0);
        assert_eq!(frame.throttle_r, 0);
        assert_eq!(frame.uptime_ms, TELEMETRY_INTERVAL_MS);
    }

    #[test]
    fn telemetry_frame_includes_lidar_readings() {
        let mut r = make_robot_with_wifi();
        r.lidar_l.set(150);
        r.lidar_r.set(80);
        for t in 0..=TELEMETRY_INTERVAL_MS {
            r.tick(t);
        }
        let frame = r.wifi.sent_frames.last().expect("a frame must have been sent");
        assert_eq!(frame.lidar_l_cm, Some(150));
        assert_eq!(frame.lidar_r_cm, Some(80));
    }

    // -------------------------------------------------------------------------
    // RobotState — name() method
    // -------------------------------------------------------------------------

    #[test]
    fn state_names_are_correct() {
        assert_eq!(RobotState::Idle.name(),     "IDLE");
        assert_eq!(RobotState::Record.name(),   "RECORD");
        assert_eq!(RobotState::Ready.name(),    "READY");
        assert_eq!(RobotState::Play.name(),     "PLAY");
        assert_eq!(RobotState::Avoiding.name(), "AVOIDING");
        assert_eq!(RobotState::Halt.name(),     "HALT");
        assert_eq!(RobotState::Direct.name(),   "DIRECT");
    }

    // -------------------------------------------------------------------------
    // DIRECT state tests
    // -------------------------------------------------------------------------

    #[test]
    fn direct_entered_from_idle_on_long_press() {
        let mut r = make_robot();

        // Simulate a long button press: press + hold.
        r.input.press_and_hold();
        r.tick(0); // press edge detected → hold_start = 0; still held → no transition yet

        // Keep ticking with button held — must not transition prematurely.
        for t in 1..LONG_PRESS_MS {
            r.tick(t);
            assert_eq!(r.state, RobotState::Idle, "must stay Idle while button is held (t={})", t);
        }

        // Release button after LONG_PRESS_MS has elapsed.
        r.input.release();
        r.tick(LONG_PRESS_MS);
        assert_eq!(r.state, RobotState::Direct, "long press must transition Idle → Direct");
    }

    #[test]
    fn direct_short_press_does_not_enter_direct() {
        // A press shorter than LONG_PRESS_MS must go to RECORD, not DIRECT.
        let mut r = make_robot();
        r.input.press(); // btn_held defaults to false → instant release
        r.tick(0);
        assert_eq!(
            r.state,
            RobotState::Record,
            "short press (btn_held=false) must enter RECORD, not DIRECT"
        );
    }

    #[test]
    fn direct_joystick_drives_motors() {
        // Build a robot already in Direct state.
        let mut r = make_robot();
        r.input.press_and_hold();
        r.tick(0);
        for t in 1..LONG_PRESS_MS {
            r.tick(t);
        }
        r.input.release();
        r.tick(LONG_PRESS_MS);
        assert_eq!(r.state, RobotState::Direct);

        // In Direct state, joystick throttle must be passed straight to motors.
        r.input.set_throttle(60, -40);
        r.tick(LONG_PRESS_MS + 1);
        assert_eq!(
            r.motors.last_drive(),
            Some((60, -40)),
            "Direct must apply joystick throttle directly to motors every tick"
        );
    }

    #[test]
    fn direct_button_press_exits_to_idle() {
        // Build a robot in Direct state.
        let mut r = make_robot();
        r.input.press_and_hold();
        r.tick(0);
        for t in 1..LONG_PRESS_MS {
            r.tick(t);
        }
        r.input.release();
        r.tick(LONG_PRESS_MS);
        assert_eq!(r.state, RobotState::Direct);

        // Any button press in Direct must exit to Idle and coast.
        let t_exit = LONG_PRESS_MS + 2;
        let coasts_before = r.motors.coasts;
        r.input.press();
        r.tick(t_exit);
        assert_eq!(r.state, RobotState::Idle, "button press in Direct must transition to Idle");
        assert!(
            r.motors.coasts > coasts_before,
            "exiting Direct must call coast() to stop motors"
        );
    }

    /// A hold released at exactly LONG_PRESS_MS - 1 ms must go to RECORD, not DIRECT.
    #[test]
    fn direct_long_press_one_ms_short_goes_to_record() {
        let mut r = make_robot();
        r.input.press_and_hold(); // edge + held
        r.tick(0); // hold_start = 0; still held → stay Idle

        // Keep button held through tick LONG_PRESS_MS - 2 (held_ms still < threshold).
        for t in 1..LONG_PRESS_MS - 1 {
            r.tick(t);
            assert_eq!(r.state, RobotState::Idle, "must stay Idle while button held (t={})", t);
        }

        // Release one tick before threshold — held_ms = (LONG_PRESS_MS-1) - 0 = LONG_PRESS_MS-1.
        r.input.release();
        r.tick(LONG_PRESS_MS - 1);
        assert_eq!(
            r.state,
            RobotState::Record,
            "hold of LONG_PRESS_MS-1 ms must go to RECORD, not DIRECT"
        );
    }

    /// The WiFi button must have no effect while in DIRECT mode.
    #[test]
    fn direct_wifi_button_ignored() {
        // Build a robot with WiFi, then enter DIRECT via long press.
        let mut r = make_robot_with_wifi();
        r.input.press_and_hold();
        r.tick(0);
        for t in 1..LONG_PRESS_MS {
            r.tick(t);
        }
        r.input.release();
        r.tick(LONG_PRESS_MS);
        assert_eq!(r.state, RobotState::Direct);

        // Fire WiFi button — tick_direct does NOT check wifi.poll_button().
        r.wifi.button_presses = 1;
        r.tick(LONG_PRESS_MS + 1);
        assert_eq!(
            r.state,
            RobotState::Direct,
            "WiFi button must be ignored in DIRECT state"
        );
    }

    /// Each tick in DIRECT applies the current joystick throttle to the motors.
    #[test]
    fn direct_throttle_changes_applied_per_tick() {
        let mut r = make_robot();
        r.input.press_and_hold();
        r.tick(0);
        for t in 1..LONG_PRESS_MS {
            r.tick(t);
        }
        r.input.release();
        r.tick(LONG_PRESS_MS);
        assert_eq!(r.state, RobotState::Direct);

        // First throttle setting.
        r.input.set_throttle(30, -30);
        r.tick(LONG_PRESS_MS + 1);
        assert_eq!(r.motors.last_drive(), Some((30, -30)));

        // Change throttle — must be reflected in the very next tick.
        r.input.set_throttle(70, 20);
        r.tick(LONG_PRESS_MS + 2);
        assert_eq!(
            r.motors.last_drive(),
            Some((70, 20)),
            "updated throttle must be applied to motors on the very next tick"
        );
    }

    /// In DIRECT mode the LCD row 1 must show live throttle values, not LIDAR
    /// readings.  In all other states it must continue to show LIDAR.
    #[test]
    fn direct_lcd_row1_shows_throttle_not_lidar() {
        let mut r = make_robot_with_display();

        // Enter DIRECT via long press.
        r.input.press_and_hold();
        r.tick(0);
        for t in 1..LONG_PRESS_MS {
            r.tick(t);
        }
        r.input.release();
        r.tick(LONG_PRESS_MS);
        assert_eq!(r.state, RobotState::Direct);

        // Set a known throttle then advance past the telemetry interval so the
        // display block fires.
        r.input.set_throttle(75, -50);
        let t_tel = LONG_PRESS_MS + TELEMETRY_INTERVAL_MS + 1;
        r.tick(t_tel);

        let row1 = r.display.last_row1().to_string();
        assert!(
            row1.contains("+75") && row1.contains("-50"),
            "DIRECT row 1 must show throttle values, got: {:?}",
            row1
        );
        assert!(
            !row1.contains("cm"),
            "DIRECT row 1 must not show LIDAR 'cm' suffix, got: {:?}",
            row1
        );
    }

    /// Outside DIRECT mode the LCD row 1 must show LIDAR readings, not throttle.
    #[test]
    fn non_direct_lcd_row1_shows_lidar_not_throttle() {
        let mut r = make_robot_with_display();

        // Remain in Idle and advance past the telemetry interval.
        r.tick(TELEMETRY_INTERVAL_MS + 1);

        let row1 = r.display.last_row1().to_string();
        assert!(
            row1.contains("cm") || row1.contains("---"),
            "Non-DIRECT row 1 must show LIDAR format, got: {:?}",
            row1
        );
        // "---" itself contains '-', so only check that no '+' sign appears
        // (throttle values always carry an explicit '+' for positive readings).
        assert!(
            !row1.contains('+'),
            "Non-DIRECT row 1 must not show signed throttle values, got: {:?}",
            row1
        );
    }

    // -------------------------------------------------------------------------
    // IDLE — second-press drop
    // -------------------------------------------------------------------------

    /// A second button press while a hold is already in progress must be silently
    /// dropped (debounce guardrail).  The existing hold_start must be preserved.
    #[test]
    fn idle_second_press_while_hold_in_progress_is_dropped() {
        let mut r = make_robot();

        // First press + hold: sets hold_start without releasing.
        r.input.press_and_hold();
        r.tick(0);
        assert_eq!(r.state, RobotState::Idle, "button still held → stay Idle");
        assert!(
            r.button_hold_start.is_some(),
            "hold_start must be set after first press"
        );

        // Enqueue a second press while the hold is still active.
        r.input.press();
        r.tick(1);
        // The second press edge should be consumed but silently dropped.
        assert_eq!(r.state, RobotState::Idle, "still Idle with hold in progress");
        assert!(
            r.button_hold_start.is_some(),
            "hold_start must survive the dropped second press"
        );
        // pending_presses must now be empty (the edge was consumed, not re-queued).
        assert_eq!(
            r.input.pending_presses,
            0,
            "dropped press must have been consumed, not re-queued"
        );
    }

    // -------------------------------------------------------------------------
    // AVOIDING Phase 3 — stale sensor stays in Avoiding
    // -------------------------------------------------------------------------

    /// None sensor readings in Phase 3 must keep the robot in Avoiding (fail-safe:
    /// a stale/missing sensor must not be treated as "path clear").
    #[test]
    fn avoiding_phase3_stale_sensor_stays_in_avoiding() {
        let mut r = robot_in_avoiding();

        // Advance into Phase 3.
        let phase3_start = r.avoid_start_ms + AVOID_BACK_MS + AVOID_TURN_MS + 1;

        // Clear both sensors to None (stale / sensor error).
        r.lidar_l.clear();
        r.lidar_r.clear();

        r.tick(phase3_start);
        assert_eq!(
            r.state,
            RobotState::Avoiding,
            "None sensor in Phase 3 must NOT be treated as 'clear'"
        );

        // A second tick must still stay in Avoiding (not resume Play).
        r.tick(phase3_start + 1);
        assert_eq!(
            r.state,
            RobotState::Avoiding,
            "Avoiding must persist with stale sensors"
        );
    }
}

