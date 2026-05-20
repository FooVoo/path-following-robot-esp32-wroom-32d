//! End-to-end FSM scenario tests — run on the host (`aarch64-apple-darwin`).
//!
//! These tests model the full robot loop in a controlled time environment.
//! Each call to `tick()` advances the simulated clock by 1 ms and drives the
//! FSM through realistic multi-state sequences.  They complement the unit
//! tests in `src/domain/robot.rs` by validating *transitions* and
//! *invariants* across state boundaries rather than individual handlers.
//!
//! ## Design
//!
//! All mocks are wrapped in `Rc<RefCell<…>>` so the test holds a shared
//! handle that it can mutate (inject sensor readings, press buttons) while
//! the `Robot` owns the same underlying state through a cheap clone of the
//! wrapper.
//!
//! ## Running
//! ```bash
//! cargo +stable test --test fsm_scenarios --target aarch64-apple-darwin
//! ```

use std::{cell::RefCell, rc::Rc};

use path_following_robot::{
    config::{AVOID_BACK_MS, AVOID_TURN_MS, LONG_PRESS_MS, OBSTACLE_CM, PATH_CMD_INTERVAL_MS},
    domain::{robot::Robot, state::RobotState},
    ports::{distance::DistancePort, input::InputPort, motors::MotorPort},
};

// ── Shared mock infrastructure ────────────────────────────────────────────────

// ----- Motors ----------------------------------------------------------------

#[derive(Default)]
struct MotorsInner {
    coasts: u32,
    drives: Vec<(i8, i8)>,
}

#[derive(Clone, Default)]
struct SharedMotors(Rc<RefCell<MotorsInner>>);

impl SharedMotors {
    fn coasts(&self) -> u32 {
        self.0.borrow().coasts
    }
    fn last_drive(&self) -> Option<(i8, i8)> {
        self.0.borrow().drives.last().copied()
    }
}

impl MotorPort for SharedMotors {
    fn drive(&mut self, left: i8, right: i8) {
        self.0.borrow_mut().drives.push((left, right));
    }
    fn coast(&mut self) {
        self.0.borrow_mut().coasts += 1;
    }
}

// ----- Distance sensor -------------------------------------------------------

#[derive(Default)]
struct DistanceInner {
    dist_cm: Option<u16>,
}

#[derive(Clone, Default)]
struct SharedDistance(Rc<RefCell<DistanceInner>>);

impl SharedDistance {
    fn set(&self, cm: u16) {
        self.0.borrow_mut().dist_cm = Some(cm);
    }
    fn clear(&self) {
        self.0.borrow_mut().dist_cm = None;
    }
}

impl DistancePort for SharedDistance {
    fn poll(&mut self) {}
    fn distance_cm(&self) -> Option<u16> {
        self.0.borrow().dist_cm
    }
    fn tick_staleness(&mut self) {}
}

// ----- Input (button + joystick) ---------------------------------------------

#[derive(Default)]
struct InputInner {
    pending_presses: usize,
    btn_held: bool,
    tl: i8,
    tr: i8,
}

#[derive(Clone, Default)]
struct SharedInput(Rc<RefCell<InputInner>>);

impl SharedInput {
    fn press(&self) {
        self.0.borrow_mut().pending_presses += 1;
    }
    fn press_and_hold(&self) {
        let mut inner = self.0.borrow_mut();
        inner.pending_presses += 1;
        inner.btn_held = true;
    }
    fn release(&self) {
        self.0.borrow_mut().btn_held = false;
    }
    fn set_throttle(&self, l: i8, r: i8) {
        let mut inner = self.0.borrow_mut();
        inner.tl = l;
        inner.tr = r;
    }
}

impl InputPort for SharedInput {
    fn poll(&mut self, _now_ms: u64) {}
    fn throttle_left(&self) -> i8 {
        self.0.borrow().tl
    }
    fn throttle_right(&self) -> i8 {
        self.0.borrow().tr
    }
    fn take_button_press(&mut self) -> bool {
        let mut inner = self.0.borrow_mut();
        if inner.pending_presses > 0 {
            inner.pending_presses -= 1;
            true
        } else {
            false
        }
    }
    fn is_button_held(&self) -> bool {
        self.0.borrow().btn_held
    }
}

// ── Test harness ──────────────────────────────────────────────────────────────

/// All test handles in one struct so scenarios don't juggle loose variables.
struct Harness {
    robot:   Robot<SharedMotors, SharedDistance, SharedInput>,
    motors:  SharedMotors,
    lidar_l: SharedDistance,
    lidar_r: SharedDistance,
    input:   SharedInput,
}

impl Harness {
    fn new() -> Self {
        let motors  = SharedMotors::default();
        let lidar_l = SharedDistance::default();
        let lidar_r = SharedDistance::default();
        let input   = SharedInput::default();
        let robot   = Robot::new(
            motors.clone(),
            lidar_l.clone(),
            lidar_r.clone(),
            input.clone(),
        );
        Self { robot, motors, lidar_l, lidar_r, input }
    }

    fn tick(&mut self, now_ms: u64) {
        self.robot.tick(now_ms);
    }

    fn tick_n(&mut self, start_ms: u64, n: u64) {
        for i in 0..n {
            self.robot.tick(start_ms + i);
        }
    }

    fn state(&self) -> RobotState {
        self.robot.state()
    }
}

// ── Scenarios ─────────────────────────────────────────────────────────────────

/// **Happy path** – IDLE → RECORD (2 commands) → READY → PLAY → HALT.
///
/// Validates the full lifecycle with two recorded motion commands.
#[test]
fn scenario_record_two_commands_play_to_halt() {
    let mut h = Harness::new();
    h.input.set_throttle(50, 50);

    assert_eq!(h.state(), RobotState::Idle);

    // Short press → RECORD
    h.input.press();
    h.tick(0);
    assert_eq!(h.state(), RobotState::Record);

    // Let two full record intervals elapse.
    let record_ticks = 2 * PATH_CMD_INTERVAL_MS;
    h.tick_n(1, record_ticks);
    assert_eq!(h.state(), RobotState::Record, "must stay in RECORD during capture");

    // Stop recording → READY
    h.input.press();
    let t_ready = record_ticks + 1;
    h.tick(t_ready);
    assert_eq!(h.state(), RobotState::Ready);

    // Start playback → PLAY
    h.input.press();
    let t_play = t_ready + 1;
    h.tick(t_play);
    assert_eq!(h.state(), RobotState::Play);

    // Advance past the recorded path duration + buffer.
    h.tick_n(t_play + 1, record_ticks + 50);
    assert_eq!(h.state(), RobotState::Halt, "must reach HALT when path exhausted");

    // Motors must have been coasted on the PLAY → HALT transition.
    assert!(h.motors.coasts() > 0, "coast must be called on HALT entry");
}

// ─────────────────────────────────────────────────────────────────────────────

/// **Avoidance cycle** – PLAY → AVOIDING (obstacle) → Phase 3 coast → PLAY (clear) → HALT.
///
/// Uses sensor injection via the `SharedDistance` handle to synthesise a left-side
/// obstacle mid-playback, then clears it in Phase 3 to verify resumption.
#[test]
fn scenario_obstacle_detected_avoided_and_resumed() {
    let mut h = Harness::new();
    h.input.set_throttle(40, 40);

    // Build a path: record one interval.
    h.input.press();
    h.tick(0);
    h.tick_n(1, PATH_CMD_INTERVAL_MS);
    h.input.press();
    let t_ready = PATH_CMD_INTERVAL_MS + 1;
    h.tick(t_ready);

    // Start playback.
    h.input.press();
    let t_play = t_ready + 1;
    h.tick(t_play);
    assert_eq!(h.state(), RobotState::Play);

    // Inject a left obstacle — robot must immediately enter AVOIDING.
    h.lidar_l.set(OBSTACLE_CM - 1);
    let t_obstacle = t_play + 1;
    h.tick(t_obstacle);
    assert_eq!(h.state(), RobotState::Avoiding, "obstacle must trigger AVOIDING");

    let avoid_start = t_obstacle; // avoid_start_ms is set in this tick

    // ── Phase 1 (reverse) ────────────────────────────────────────────────────
    let phase1_mid = avoid_start + AVOID_BACK_MS / 2;
    h.tick(phase1_mid);
    assert_eq!(h.state(), RobotState::Avoiding, "must stay in Phase 1");
    let last_drive = h.motors.last_drive().expect("motors must be driven in Phase 1");
    assert!(
        last_drive.0 < 0 && last_drive.1 < 0,
        "Phase 1 must drive both motors backward (got {:?})",
        last_drive
    );

    // ── Phase 2 (turn) ────────────────────────────────────────────────────────
    let phase2_mid = avoid_start + AVOID_BACK_MS + AVOID_TURN_MS / 2;
    h.tick(phase2_mid);
    assert_eq!(h.state(), RobotState::Avoiding, "must stay in Phase 2");

    // ── Phase 3 (wait with motors coasted) ────────────────────────────────────
    let phase3_start = avoid_start + AVOID_BACK_MS + AVOID_TURN_MS + 1;
    let coasts_before = h.motors.coasts();
    h.lidar_l.set(OBSTACLE_CM - 1); // still blocked
    h.lidar_r.clear();              // right sensor has no reading (None → not clear)
    h.tick(phase3_start);
    assert_eq!(h.state(), RobotState::Avoiding, "should stay in Phase 3");
    assert!(
        h.motors.coasts() > coasts_before,
        "motors must be coasted at Phase 3 entry"
    );

    // ── Sensor clears → resume PLAY ───────────────────────────────────────────
    use path_following_robot::config::CLEAR_CM;
    h.lidar_l.set(CLEAR_CM);
    h.lidar_r.set(CLEAR_CM);
    h.tick(phase3_start + 1);
    assert_eq!(
        h.state(),
        RobotState::Play,
        "clearing both sensors must resume PLAY"
    );

    // Play must finish normally → HALT.
    h.tick_n(phase3_start + 2, PATH_CMD_INTERVAL_MS + 50);
    assert_eq!(h.state(), RobotState::Halt, "must halt after path completes post-avoidance");
}

// ─────────────────────────────────────────────────────────────────────────────

/// **DIRECT mode** – IDLE → DIRECT (long press) → manual drive → IDLE (exit) → RECORD.
///
/// Verifies that:
/// 1. Hold for LONG_PRESS_MS enters DIRECT.
/// 2. Joystick throttle is mirrored to motors every tick in DIRECT.
/// 3. A button press from DIRECT returns to IDLE and coasts motors.
/// 4. The robot can immediately proceed to RECORD from IDLE after exiting DIRECT.
#[test]
fn scenario_direct_mode_drive_then_exit_and_record() {
    let mut h = Harness::new();

    // ── Enter DIRECT ──────────────────────────────────────────────────────────
    h.input.press_and_hold();
    h.tick(0);
    for t in 1..LONG_PRESS_MS {
        h.tick(t);
        assert_eq!(h.state(), RobotState::Idle, "must stay IDLE while held (t={})", t);
    }
    h.input.release();
    h.tick(LONG_PRESS_MS);
    assert_eq!(h.state(), RobotState::Direct, "long press must enter DIRECT");

    // ── Manual drive ─────────────────────────────────────────────────────────
    h.input.set_throttle(70, -70);
    let t0 = LONG_PRESS_MS + 1;
    h.tick(t0);
    assert_eq!(
        h.motors.last_drive(),
        Some((70, -70)),
        "DIRECT must apply throttle directly to motors"
    );

    // Different throttle on next tick.
    h.input.set_throttle(0, 0);
    h.tick(t0 + 1);
    assert_eq!(
        h.motors.last_drive(),
        Some((0, 0)),
        "throttle changes must be applied each tick"
    );

    // ── Exit DIRECT → IDLE ────────────────────────────────────────────────────
    let coasts_before = h.motors.coasts();
    h.input.press();
    h.tick(t0 + 2);
    assert_eq!(h.state(), RobotState::Idle, "button press must exit DIRECT to IDLE");
    assert!(h.motors.coasts() > coasts_before, "must coast on DIRECT exit");

    // ── Short press → RECORD (reuse the same IDLE) ────────────────────────────
    h.input.set_throttle(30, 30);
    h.input.press();
    let t_record = t0 + 3;
    h.tick(t_record);
    assert_eq!(h.state(), RobotState::Record, "short press must enter RECORD after DIRECT");
}

// ─────────────────────────────────────────────────────────────────────────────

/// **Avoiding timeout** – Phase 3 sensor permanently blocked → HALT after AVOID_TIMEOUT_MS.
#[test]
fn scenario_avoiding_phase3_timeout_to_halt() {
    let mut h = Harness::new();
    h.input.set_throttle(50, 50);

    // Record and start play.
    h.input.press();
    h.tick(0);
    h.tick_n(1, PATH_CMD_INTERVAL_MS);
    h.input.press();
    let t_ready = PATH_CMD_INTERVAL_MS + 1;
    h.tick(t_ready);
    h.input.press();
    let t_play = t_ready + 1;
    h.tick(t_play);

    // Trigger obstacle.
    h.lidar_l.set(OBSTACLE_CM - 1);
    let t_avoid = t_play + 1;
    h.tick(t_avoid);
    assert_eq!(h.state(), RobotState::Avoiding);

    let avoid_start = t_avoid;

    // Keep sensors blocked through all three phases.
    // Advance to just inside Phase 3 timeout.
    let phase3_entry = avoid_start + AVOID_BACK_MS + AVOID_TURN_MS + 1;
    h.tick(phase3_entry);
    assert_eq!(h.state(), RobotState::Avoiding, "should be in Phase 3");

    // Advance to exactly the timeout boundary; sensors still blocked.
    let timeout_tick = avoid_start + path_following_robot::config::AVOID_TIMEOUT_MS;
    // Drive up to (not including) timeout.
    let ticks_to_timeout = timeout_tick - phase3_entry - 1;
    h.tick_n(phase3_entry + 1, ticks_to_timeout);
    assert_eq!(h.state(), RobotState::Avoiding, "must stay AVOIDING until timeout");

    // One more tick crosses the timeout.
    h.tick(timeout_tick);
    assert_eq!(h.state(), RobotState::Halt, "timeout must trigger HALT");
}

// ─────────────────────────────────────────────────────────────────────────────

/// **Short press vs long press boundary** – exactly LONG_PRESS_MS - 1 ms hold → RECORD.
#[test]
fn scenario_press_boundary_short_goes_to_record_not_direct() {
    let mut h = Harness::new();

    // Hold for exactly LONG_PRESS_MS - 1 ms (one ms below the long-press threshold).
    h.input.press_and_hold();
    h.tick(0);
    for t in 1..LONG_PRESS_MS - 1 {
        h.tick(t);
    }
    // Release just under the threshold.
    h.input.release();
    h.tick(LONG_PRESS_MS - 1);
    assert_eq!(
        h.state(),
        RobotState::Record,
        "hold shorter than LONG_PRESS_MS must enter RECORD, not DIRECT"
    );
}

// ─────────────────────────────────────────────────────────────────────────────

/// `Robot::state()` must return `Idle` immediately after construction.
#[test]
fn initial_state_is_idle() {
    let h = Harness::new();
    assert_eq!(h.state(), RobotState::Idle);
}
