//! WiFi adapter — implements [`RemoteControlPort`] + [`TelemetryPort`] over UDP.
//!
//! # Architecture
//!
//! ```text
//!  Robot::tick()
//!    └─ wifi.poll_network(now_ms)       ← pumps smoltcp + drains UDP socket
//!         └─ parse 4-byte command       ← fills pending_throttle / pending_button
//!
//!  Robot::tick() (telemetry path)
//!    └─ wifi.send(&frame)               ← rate-limited UDP broadcast of JSON
//!
//!  Robot::tick_idle / tick_ready
//!    └─ wifi.poll_button()              ← consume pending remote button
//!
//!  Robot::tick_record
//!    └─ wifi.poll_throttle()            ← consume pending remote throttle
//! ```
//!
//! # Wire protocol
//!
//! **Commands → robot (UDP port [`WIFI_CMD_PORT`]), 4 bytes:**
//!
//! ```text
//! [0xA5, type, v1, v2]
//!   0x01  throttle   — v1 = left as i8 (cast from u8), v2 = right as i8
//!   0x02  button     — v1, v2 ignored
//! ```
//!
//! **Telemetry ← robot (UDP limited-broadcast port [`WIFI_TEL_PORT`]), ~100 bytes JSON:**
//!
//! ```text
//! {"s":"PLAY","ll":125,"lr":98,"tl":50,"tr":50,"ms":12345,"ip":"192.168.1.42"}
//! ```
//!
//! `ll`/`lr` = LIDAR distances in cm, or -1 when the sensor is stale/absent.  
//! `tl`/`tr` = last motor throttle in \[-100, 100\].  
//! `ms`      = uptime in milliseconds.
//! `ip`      = robot's current IPv4 address (DHCP-assigned).
//!
//! # IP address
//!
//! The robot obtains its IP via DHCP on startup.  The assigned address is
//! embedded in every telemetry frame as the `"ip"` field, allowing the
//! host-side `telemetry-server` to discover the robot dynamically without
//! any manual configuration.  Telemetry frames are broadcast to
//! `255.255.255.255` (limited broadcast) so they reach all hosts on the
//! local subnet regardless of the robot's assigned address.
//!
//! [`WIFI_CMD_PORT`]: crate::config::WIFI_CMD_PORT
//! [`WIFI_TEL_PORT`]: crate::config::WIFI_TEL_PORT

extern crate alloc;

use alloc::vec;

use core::{
    fmt::Write as FmtWrite,
    future::Future,
    pin::Pin,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use esp_hal::{peripherals::WIFI, time::Instant};
use esp_radio::wifi::{
    Config, ControllerConfig, Interface as RadioInterface, WifiController, WifiRxToken, WifiTxToken,
    sta::StationConfig,
};
use heapless::String as HString;
use log::{error, info, trace, warn};
use smoltcp::{
    iface::{Config as SmoltcpConfig, Interface, SocketHandle, SocketSet},
    phy::{Device, DeviceCapabilities, Medium},
    socket::{
        dhcpv4,
        udp::{PacketBuffer, PacketMetadata, Socket as UdpSocket},
    },
    time::Instant as SmoltcpInstant,
    wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address},
};

use crate::{
    config::{
        TELEMETRY_INTERVAL_MS, WIFI_CMD_PORT, WIFI_DHCP_TIMEOUT_MS, WIFI_PASSWORD, WIFI_SSID,
        WIFI_TEL_PORT,
    },
    ports::{
        remote_control::RemoteControlPort,
        telemetry::{TelemetryFrame, TelemetryPort},
    },
};

// ── Minimal block_on for no_std / no-executor ─────────────────────────────────
//
// Spins a future to completion using a no-op waker.  Safe to use here because
// esp-rtos has already been started, so WiFi ISR tasks run in the background
// and will advance the `connect_async()` future without the caller needing to
// yield.
fn block_on<F: Future>(mut f: F) -> F::Output {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |p| RawWaker::new(p, &VTABLE), // clone
        |_| {},                        // wake
        |_| {},                        // wake_by_ref
        |_| {},                        // drop
    );
    let raw = RawWaker::new(core::ptr::null(), &VTABLE);
    // SAFETY: vtable operations are all no-ops; no state is accessed.
    let waker = unsafe { Waker::from_raw(raw) };
    let mut cx = Context::from_waker(&waker);
    // SAFETY: `f` is never moved after this point.
    let mut f = unsafe { Pin::new_unchecked(&mut f) };
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => core::hint::spin_loop(),
        }
    }
}

// ── smoltcp Device bridge ────────────────────────────────────────────────────
//
// `esp_radio::wifi::Interface<'static>` (tied to the `'static` WIFI peripheral)
// exposes sync `receive()` / `transmit()` methods that return token options
// without needing an async waker.  We wrap those tokens in newtype wrappers
// that implement the `smoltcp::phy::{RxToken, TxToken}` traits.
//
// smoltcp's `RxToken::consume` passes `&[u8]` (immutable) while
// embassy-net-driver / esp-radio uses `&mut [u8]`.  We bridge via a closure
// that coerces `&mut [u8]` → `&[u8]`.

struct WifiRxWrapper(WifiRxToken);
struct WifiTxWrapper(WifiTxToken);

impl smoltcp::phy::RxToken for WifiRxWrapper {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        // WifiRxToken::consume_token requires FnOnce(&mut [u8]) -> R.
        // smoltcp provides FnOnce(&[u8]) -> R.
        // &mut [u8] coerces to &[u8], so the closure wrapper is valid.
        self.0.consume_token(|buf: &mut [u8]| f(buf))
    }
}

impl smoltcp::phy::TxToken for WifiTxWrapper {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        self.0.consume_token(len, f)
    }
}

struct WifiBridge(RadioInterface<'static>);

impl Device for WifiBridge {
    type RxToken<'a>
        = WifiRxWrapper
    where
        Self: 'a;
    type TxToken<'a>
        = WifiTxWrapper
    where
        Self: 'a;

    fn receive(&mut self, _: SmoltcpInstant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.0.receive().map(|(rx, tx)| (WifiRxWrapper(rx), WifiTxWrapper(tx)))
    }

    fn transmit(&mut self, _: SmoltcpInstant) -> Option<Self::TxToken<'_>> {
        self.0.transmit().map(WifiTxWrapper)
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps
    }
}

// ── Public WiFi adapter ───────────────────────────────────────────────────────

/// Concrete WiFi adapter.
///
/// `inner` is `None` when WiFi failed during [`WifiAdapter::connect`].  In
/// that case, all port methods are silent no-ops and the robot operates
/// without remote control or telemetry.
pub struct WifiAdapter {
    inner: Option<WifiAdapterInner>,
}

// Fields are pub(crate) only to allow the integration test to inspect state
// if needed; they are never accessed from outside this module in normal use.
struct WifiAdapterInner {
    // `_controller` must be kept alive to maintain the WiFi connection.
    _controller: WifiController<'static>,
    iface: Interface,
    bridge: WifiBridge,
    sockets: SocketSet<'static>,
    cmd_handle: SocketHandle,
    tel_handle: SocketHandle,
    /// Most-recent remote throttle command; consumed by `poll_throttle()`.
    pending_throttle: Option<(i8, i8)>,
    /// Pending remote button press; consumed by `poll_button()`.
    pending_button: bool,
    /// `uptime_ms` when the last telemetry frame was sent (rate-limiting).
    last_tel_ms: u64,
    /// IPv4 address assigned by DHCP; embedded in every telemetry frame.
    assigned_ip: [u8; 4],
}

impl WifiAdapter {
    // -----------------------------------------------------------------------
    // Construction / connection
    // -----------------------------------------------------------------------

    /// Initialise the WiFi peripheral, obtain an IP via DHCP, and allocate
    /// two UDP sockets (command + telemetry).
    ///
    /// Blocks until the AP association and DHCP exchange both succeed.  On any
    /// failure, logs the error and returns an adapter in "offline" mode — all
    /// port methods become no-ops.
    ///
    /// **Note:** `esp_rtos::start()` **must** have been called before this
    /// function, and `esp_alloc::heap_allocator!(size: …)` must have been
    /// called to supply the smoltcp heap.
    pub fn connect(wifi: WIFI<'static>) -> Self {
        match Self::try_connect(wifi) {
            Ok(inner) => {
                let [a, b, c, d] = inner.assigned_ip;
                info!(
                    "WiFi: connected — IP {}.{}.{}.{} — remote control + telemetry enabled",
                    a, b, c, d
                );
                Self { inner: Some(inner) }
            }
            Err(()) => {
                error!("WiFi: connect failed — robot operating offline (no remote / telemetry)");
                Self { inner: None }
            }
        }
    }

    fn try_connect(wifi: WIFI<'static>) -> Result<WifiAdapterInner, ()> {
        // ── 1. Create WiFi controller with station credentials ───────────────
        //
        // `esp_rtos::start()` must already be running — it installs the ISR
        // handlers that drive WiFi background tasks and make `connect_async()`
        // actually complete.
        let station_config = Config::Station(
            StationConfig::default()
                .with_ssid(WIFI_SSID)
                .with_password(WIFI_PASSWORD.into()),
        );

        let (mut controller, interfaces) =
            esp_radio::wifi::new(wifi, ControllerConfig::default().with_initial_config(station_config))
                .map_err(|e| error!("WiFi: esp_radio::wifi::new error: {:?}", e))?;

        // ── 2. Connect (truly async; block_on spins while ISRs drive it) ─────
        info!("WiFi: connecting to SSID '{}'", WIFI_SSID);
        block_on(controller.connect_async())
            .map_err(|e| error!("WiFi: connect_async error: {:?}", e))?;
        info!("WiFi: associated with AP");

        // ── 3. Build bare smoltcp interface (no IP yet — DHCP assigns it) ────
        let radio_iface = interfaces.station;
        let mac = radio_iface.mac_address();
        let mut bridge = WifiBridge(radio_iface);
        let hw_addr = HardwareAddress::Ethernet(EthernetAddress(mac));
        let smoltcp_now = SmoltcpInstant::from_millis(
            Instant::now().duration_since_epoch().as_millis() as i64,
        );
        let mut iface = Interface::new(SmoltcpConfig::new(hw_addr), &mut bridge, smoltcp_now);

        // ── 4. Obtain IP via DHCP ─────────────────────────────────────────────
        //
        // A temporary SocketSet is used for the DHCP exchange only; it is
        // dropped once we have a lease, keeping the main socket set clean.
        let assigned_ip: [u8; 4] = {
            let mut dhcp_sockets: SocketSet<'static> = SocketSet::new(vec![]);
            let dhcp_socket = dhcpv4::Socket::new();
            let dhcp_handle = dhcp_sockets.add(dhcp_socket);

            info!("WiFi: waiting for DHCP lease (timeout {}ms)…", WIFI_DHCP_TIMEOUT_MS);
            let dhcp_start_ms = Instant::now().duration_since_epoch().as_millis();

            loop {
                let now_ms = Instant::now().duration_since_epoch().as_millis();
                if now_ms.saturating_sub(dhcp_start_ms) > WIFI_DHCP_TIMEOUT_MS {
                    error!("WiFi: DHCP timed out after {}ms", WIFI_DHCP_TIMEOUT_MS);
                    return Err(());
                }

                let smoltcp_ts = SmoltcpInstant::from_millis(now_ms as i64);
                iface.poll(smoltcp_ts, &mut bridge, &mut dhcp_sockets);

                let event = dhcp_sockets
                    .get_mut::<dhcpv4::Socket>(dhcp_handle)
                    .poll();

                if let Some(dhcpv4::Event::Configured(cfg)) = event {
                    let cidr = cfg.address;
                    let ip = cidr.address();

                    // Apply the assigned address and gateway to the interface.
                    iface.update_ip_addrs(|ips| {
                        ips.push(IpCidr::new(
                            IpAddress::Ipv4(ip),
                            cidr.prefix_len(),
                        ))
                        .ok();
                    });
                    if let Some(gw) = cfg.router {
                        iface.routes_mut().add_default_ipv4_route(gw).ok();
                    }

                    let [a, b, c, d] = ip.0;
                    info!(
                        "WiFi: DHCP OK — {}.{}.{}.{}/{} gw {:?}",
                        a, b, c, d, cidr.prefix_len(), cfg.router
                    );
                    break ip.0;
                }

                core::hint::spin_loop();
            }
        }; // dhcp_sockets dropped here

        // ── 5. Create UDP sockets (heap-backed via esp-alloc) ─────────────────
        //
        // smoltcp 0.12 accepts `Vec<T>` through `ManagedSlice::Owned`.
        // The heap is available because `esp_alloc::heap_allocator!` is
        // called at the start of `main()`.

        // Receive socket for 4-byte command packets.
        let cmd_rx_buf = PacketBuffer::new(vec![PacketMetadata::EMPTY; 8], vec![0u8; 256]);
        let cmd_tx_buf = PacketBuffer::new(vec![PacketMetadata::EMPTY; 1], vec![0u8; 64]);
        let cmd_socket = UdpSocket::new(cmd_rx_buf, cmd_tx_buf);

        // Send socket for ~100-byte JSON telemetry frames.
        let tel_rx_buf = PacketBuffer::new(vec![PacketMetadata::EMPTY; 1], vec![0u8; 64]);
        let tel_tx_buf = PacketBuffer::new(vec![PacketMetadata::EMPTY; 4], vec![0u8; 512]);
        let tel_socket = UdpSocket::new(tel_rx_buf, tel_tx_buf);

        let mut sockets: SocketSet<'static> = SocketSet::new(vec![]);
        let cmd_handle = sockets.add(cmd_socket);
        let tel_handle = sockets.add(tel_socket);

        // Bind sockets to their respective ports.
        sockets
            .get_mut::<UdpSocket>(cmd_handle)
            .bind(WIFI_CMD_PORT)
            .map_err(|e| error!("WiFi: bind cmd socket error: {:?}", e))?;

        // Bind telemetry socket to a port so smoltcp accepts outbound traffic.
        sockets
            .get_mut::<UdpSocket>(tel_handle)
            .bind(WIFI_TEL_PORT)
            .map_err(|e| error!("WiFi: bind tel socket error: {:?}", e))?;

        info!(
            "WiFi: UDP cmd=:{} tel=:{} bcast=255.255.255.255",
            WIFI_CMD_PORT, WIFI_TEL_PORT,
        );

        Ok(WifiAdapterInner {
            _controller: controller,
            iface,
            bridge,
            sockets,
            cmd_handle,
            tel_handle,
            pending_throttle: None,
            pending_button: false,
            last_tel_ms: 0,
            assigned_ip,
        })
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Pump the smoltcp stack and drain the command socket.
    fn poll_stack(inner: &mut WifiAdapterInner, now_ms: u64) {
        let smoltcp_now = smoltcp::time::Instant::from_millis(now_ms as i64);

        // One smoltcp poll tick — processes pending Ethernet frames.
        inner
            .iface
            .poll(smoltcp_now, &mut inner.bridge, &mut inner.sockets);

        // Drain incoming command packets.
        let mut buf = [0u8; 64];
        loop {
            let sock = inner.sockets.get_mut::<UdpSocket>(inner.cmd_handle);
            if !sock.can_recv() {
                break;
            }
            match sock.recv_slice(&mut buf) {
                Ok((4, _endpoint)) if buf[0] == 0xA5 => match buf[1] {
                    0x01 => {
                        // Throttle — v1/v2 transmitted as u8, reinterpreted as i8.
                        let tl = buf[2] as i8;
                        let tr = buf[3] as i8;
                        trace!("WiFi CMD throttle L={:+} R={:+}", tl, tr);
                        inner.pending_throttle = Some((tl, tr));
                    }
                    0x02 => {
                        trace!("WiFi CMD button press");
                        inner.pending_button = true;
                    }
                    t => warn!("WiFi CMD: unknown type {:#04x}", t),
                },
                Ok((len, _)) => warn!("WiFi CMD: unexpected packet length {}", len),
                Err(_) => break,
            }
        }
    }

    /// Format and emit a telemetry JSON frame via UDP limited broadcast.
    ///
    /// Broadcasts to `255.255.255.255` so the frame reaches the host-side
    /// `telemetry-server` on the same LAN regardless of subnet configuration.
    /// The robot's DHCP-assigned IP is embedded in the `"ip"` field so the
    /// server can discover the robot dynamically.
    fn send_telemetry(inner: &mut WifiAdapterInner, frame: &TelemetryFrame) {
        // Rate-limit: do not send more often than TELEMETRY_INTERVAL_MS.
        if frame.uptime_ms.saturating_sub(inner.last_tel_ms) < TELEMETRY_INTERVAL_MS {
            return;
        }

        // Build compact JSON in a stack-allocated 256-byte string.
        // Worst-case size: ~99 chars (AVOIDING state + max u64 uptime + IPv4).
        let mut json: HString<256> = HString::new();
        let ll = frame.lidar_l_cm.map_or(-1i32, |v| v as i32);
        let lr = frame.lidar_r_cm.map_or(-1i32, |v| v as i32);
        let [ia, ib, ic, id] = inner.assigned_ip;
        if write!(
            json,
            r#"{{"s":"{}","ll":{},"lr":{},"tl":{},"tr":{},"ms":{},"ip":"{}.{}.{}.{}"}}"#,
            frame.state_name,
            ll,
            lr,
            frame.throttle_l,
            frame.throttle_r,
            frame.uptime_ms,
            ia, ib, ic, id,
        )
        .is_err()
        {
            warn!("WiFi TEL: JSON format overflow");
            return;
        }

        // Send to limited broadcast — works on any subnet without knowing the
        // broadcast address ahead of time.
        let dest: IpEndpoint = IpEndpoint {
            addr: IpAddress::Ipv4(Ipv4Address::BROADCAST),
            port: WIFI_TEL_PORT,
        };

        let sock = inner.sockets.get_mut::<UdpSocket>(inner.tel_handle);
        if sock.can_send() {
            match sock.send_slice(json.as_bytes(), dest) {
                Ok(()) => {
                    trace!("WiFi TEL: {}", json.as_str());
                    inner.last_tel_ms = frame.uptime_ms;
                }
                Err(e) => warn!("WiFi TEL: send_slice error: {:?}", e),
            }
        } else {
            warn!("WiFi TEL: send buffer full, frame dropped");
        }
    }
}

// ── Port trait implementations ────────────────────────────────────────────────

impl RemoteControlPort for WifiAdapter {
    /// Pump the network stack and parse incoming command packets.
    ///
    /// Must be called once per robot tick so that smoltcp drains the driver
    /// Rx queue and any pending command is buffered for `poll_throttle()` /
    /// `poll_button()`.
    fn poll_network(&mut self, now_ms: u64) {
        if let Some(inner) = &mut self.inner {
            Self::poll_stack(inner, now_ms);
        }
    }

    /// Return and consume the most-recent remote throttle command, if any.
    fn poll_throttle(&mut self) -> Option<(i8, i8)> {
        self.inner.as_mut().and_then(|i| i.pending_throttle.take())
    }

    /// Return and consume a pending remote button event.
    fn poll_button(&mut self) -> bool {
        self.inner
            .as_mut()
            .map(|i| {
                let pressed = i.pending_button;
                i.pending_button = false;
                pressed
            })
            .unwrap_or(false)
    }
}

impl TelemetryPort for WifiAdapter {
    /// Broadcast a telemetry JSON frame over UDP.
    ///
    /// Rate-limited internally to [`TELEMETRY_INTERVAL_MS`]; calling more
    /// frequently is harmless.  The frame includes the robot's DHCP-assigned
    /// IP so the host server can discover the robot automatically.
    ///
    /// [`TELEMETRY_INTERVAL_MS`]: crate::config::TELEMETRY_INTERVAL_MS
    fn send(&mut self, frame: &TelemetryFrame) {
        if let Some(inner) = &mut self.inner {
            Self::send_telemetry(inner, frame);
        }
    }
}
//!
//! # Architecture
//!
//! ```text
//!  Robot::tick()
//!    └─ wifi.poll_network(now_ms)       ← pumps smoltcp + drains UDP socket
//!         └─ parse 4-byte command       ← fills pending_throttle / pending_button
//!
//!  Robot::tick() (telemetry path)
//!    └─ wifi.send(&frame)               ← rate-limited UDP broadcast of JSON
//!
//!  Robot::tick_idle / tick_ready
//!    └─ wifi.poll_button()              ← consume pending remote button
//!
//!  Robot::tick_record
//!    └─ wifi.poll_throttle()            ← consume pending remote throttle
//! ```
//!
//! # Wire protocol
//!
//! **Commands → robot (UDP port [`WIFI_CMD_PORT`]), 4 bytes:**
//!
//! ```text
//! [0xA5, type, v1, v2]
//!   0x01  throttle   — v1 = left as i8 (cast from u8), v2 = right as i8
//!   0x02  button     — v1, v2 ignored
//! ```
//!
//! **Telemetry ← robot (UDP broadcast port [`WIFI_TEL_PORT`]), ~100 bytes JSON:**
//!
//! ```text
//! {"s":"PLAY","ll":125,"lr":98,"tl":50,"tr":50,"ms":12345}
//! ```
//!
//! `ll`/`lr` = LIDAR distances in cm, or -1 when the sensor is stale/absent.  
//! `tl`/`tr` = last motor throttle in \[-100, 100\].  
//! `ms`      = uptime in milliseconds.
//!
//! [`WIFI_CMD_PORT`]: crate::config::WIFI_CMD_PORT
//! [`WIFI_TEL_PORT`]: crate::config::WIFI_TEL_PORT

extern crate alloc;

use alloc::vec;

use core::{
    fmt::Write as FmtWrite,
    future::Future,
    pin::Pin,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use esp_hal::{peripherals::WIFI, time::Instant};
use esp_radio::wifi::{
    Config, ControllerConfig, Interface as RadioInterface, WifiController, WifiRxToken, WifiTxToken,
    sta::StationConfig,
};
use heapless::String as HString;
use log::{error, info, trace, warn};
use smoltcp::{
    iface::{Config as SmoltcpConfig, Interface, SocketHandle, SocketSet},
    phy::{Device, DeviceCapabilities, Medium},
    socket::udp::{PacketBuffer, PacketMetadata, Socket as UdpSocket},
    time::Instant as SmoltcpInstant,
    wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address},
};

use crate::{
    config::{
        TELEMETRY_INTERVAL_MS, WIFI_BROADCAST_IP, WIFI_CMD_PORT, WIFI_GATEWAY,
        WIFI_PASSWORD, WIFI_SSID, WIFI_STATIC_IP, WIFI_SUBNET_PREFIX, WIFI_TEL_PORT,
    },
    ports::{
        remote_control::RemoteControlPort,
        telemetry::{TelemetryFrame, TelemetryPort},
    },
};

// ── Minimal block_on for no_std / no-executor ─────────────────────────────────
//
// Spins a future to completion using a no-op waker.  Safe to use here because
// esp-rtos has already been started, so WiFi ISR tasks run in the background
// and will advance the `connect_async()` future without the caller needing to
// yield.
fn block_on<F: Future>(mut f: F) -> F::Output {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |p| RawWaker::new(p, &VTABLE), // clone
        |_| {},                        // wake
        |_| {},                        // wake_by_ref
        |_| {},                        // drop
    );
    let raw = RawWaker::new(core::ptr::null(), &VTABLE);
    // SAFETY: vtable operations are all no-ops; no state is accessed.
    let waker = unsafe { Waker::from_raw(raw) };
    let mut cx = Context::from_waker(&waker);
    // SAFETY: `f` is never moved after this point.
    let mut f = unsafe { Pin::new_unchecked(&mut f) };
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => core::hint::spin_loop(),
        }
    }
}

// ── smoltcp Device bridge ────────────────────────────────────────────────────
//
// `esp_radio::wifi::Interface<'static>` (tied to the `'static` WIFI peripheral)
// exposes sync `receive()` / `transmit()` methods that return token options
// without needing an async waker.  We wrap those tokens in newtype wrappers
// that implement the `smoltcp::phy::{RxToken, TxToken}` traits.
//
// smoltcp's `RxToken::consume` passes `&[u8]` (immutable) while
// embassy-net-driver / esp-radio uses `&mut [u8]`.  We bridge via a closure
// that coerces `&mut [u8]` → `&[u8]`.

struct WifiRxWrapper(WifiRxToken);
struct WifiTxWrapper(WifiTxToken);

impl smoltcp::phy::RxToken for WifiRxWrapper {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        // WifiRxToken::consume_token requires FnOnce(&mut [u8]) -> R.
        // smoltcp provides FnOnce(&[u8]) -> R.
        // &mut [u8] coerces to &[u8], so the closure wrapper is valid.
        self.0.consume_token(|buf: &mut [u8]| f(buf))
    }
}

impl smoltcp::phy::TxToken for WifiTxWrapper {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        self.0.consume_token(len, f)
    }
}

struct WifiBridge(RadioInterface<'static>);

impl Device for WifiBridge {
    type RxToken<'a>
        = WifiRxWrapper
    where
        Self: 'a;
    type TxToken<'a>
        = WifiTxWrapper
    where
        Self: 'a;

    fn receive(&mut self, _: SmoltcpInstant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.0.receive().map(|(rx, tx)| (WifiRxWrapper(rx), WifiTxWrapper(tx)))
    }

    fn transmit(&mut self, _: SmoltcpInstant) -> Option<Self::TxToken<'_>> {
        self.0.transmit().map(WifiTxWrapper)
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps
    }
}

// ── Public WiFi adapter ───────────────────────────────────────────────────────

/// Concrete WiFi adapter.
///
/// `inner` is `None` when WiFi failed during [`WifiAdapter::connect`].  In
/// that case, all port methods are silent no-ops and the robot operates
/// without remote control or telemetry.
pub struct WifiAdapter {
    inner: Option<WifiAdapterInner>,
}

// Fields are pub(crate) only to allow the integration test to inspect state
// if needed; they are never accessed from outside this module in normal use.
struct WifiAdapterInner {
    // `_controller` must be kept alive to maintain the WiFi connection.
    _controller: WifiController<'static>,
    iface: Interface,
    bridge: WifiBridge,
    sockets: SocketSet<'static>,
    cmd_handle: SocketHandle,
    tel_handle: SocketHandle,
    /// Most-recent remote throttle command; consumed by `poll_throttle()`.
    pending_throttle: Option<(i8, i8)>,
    /// Pending remote button press; consumed by `poll_button()`.
    pending_button: bool,
    /// `uptime_ms` when the last telemetry frame was sent (rate-limiting).
    last_tel_ms: u64,
}

impl WifiAdapter {
    // -----------------------------------------------------------------------
    // Construction / connection
    // -----------------------------------------------------------------------

    /// Initialise the WiFi peripheral, connect to the configured AP, set a
    /// static IP, and allocate two UDP sockets (command + telemetry).
    ///
    /// Blocks until the AP association succeeds.  On failure, logs the error
    /// and returns an adapter in "offline" mode — all port methods become no-ops.
    ///
    /// **Note:** `esp_rtos::start()` **must** have been called before this
    /// function, and `esp_alloc::heap_allocator!(size: …)` must have been
    /// called to supply the smoltcp heap.
    pub fn connect(wifi: WIFI<'static>) -> Self {
        match Self::try_connect(wifi) {
            Ok(inner) => {
                info!("WiFi: connected — remote control + telemetry enabled");
                Self { inner: Some(inner) }
            }
            Err(()) => {
                error!("WiFi: connect failed — robot operating offline (no remote / telemetry)");
                Self { inner: None }
            }
        }
    }

    fn try_connect(wifi: WIFI<'static>) -> Result<WifiAdapterInner, ()> {
        // ── 1. Create WiFi controller with station credentials ───────────────
        //
        // `esp_rtos::start()` must already be running — it installs the ISR
        // handlers that drive WiFi background tasks and make `connect_async()`
        // actually complete.
        let station_config = Config::Station(
            StationConfig::default()
                .with_ssid(WIFI_SSID)
                .with_password(WIFI_PASSWORD.into()),
        );

        let (mut controller, interfaces) =
            esp_radio::wifi::new(wifi, ControllerConfig::default().with_initial_config(station_config))
                .map_err(|e| error!("WiFi: esp_radio::wifi::new error: {:?}", e))?;

        // ── 2. Connect (truly async; block_on spins while ISRs drive it) ─────
        info!("WiFi: connecting to SSID '{}'", WIFI_SSID);
        block_on(controller.connect_async())
            .map_err(|e| error!("WiFi: connect_async error: {:?}", e))?;
        info!("WiFi: associated with AP");

        // ── 3. Build smoltcp interface with static IP ─────────────────────────
        let radio_iface = interfaces.station;
        let mac = radio_iface.mac_address();
        let mut bridge = WifiBridge(radio_iface);
        let hw_addr = HardwareAddress::Ethernet(EthernetAddress(mac));
        let smoltcp_now = smoltcp::time::Instant::from_millis(
            Instant::now().duration_since_epoch().as_millis() as i64,
        );
        let mut iface = Interface::new(SmoltcpConfig::new(hw_addr), &mut bridge, smoltcp_now);

        let [a, b, c, d] = WIFI_STATIC_IP;
        let [ga, gb, gc, gd] = WIFI_GATEWAY;
        iface.update_ip_addrs(|ips| {
            ips.push(IpCidr::new(
                IpAddress::Ipv4(Ipv4Address::new(a, b, c, d)),
                WIFI_SUBNET_PREFIX,
            ))
            .ok();
        });
        iface
            .routes_mut()
            .add_default_ipv4_route(Ipv4Address::new(ga, gb, gc, gd))
            .ok();
        info!(
            "WiFi: static IP {}.{}.{}.{}/{} gw {}.{}.{}.{}",
            a, b, c, d, WIFI_SUBNET_PREFIX, ga, gb, gc, gd
        );

        // ── 6. Create UDP sockets (heap-backed via esp-alloc) ─────────────────
        //
        // smoltcp 0.12 accepts `Vec<T>` through `ManagedSlice::Owned`.
        // The heap is available because `esp_alloc::heap_allocator!` is
        // called at the start of `main()`.

        // Receive socket for 4-byte command packets.
        let cmd_rx_buf = PacketBuffer::new(vec![PacketMetadata::EMPTY; 8], vec![0u8; 256]);
        let cmd_tx_buf = PacketBuffer::new(vec![PacketMetadata::EMPTY; 1], vec![0u8; 64]);
        let cmd_socket = UdpSocket::new(cmd_rx_buf, cmd_tx_buf);

        // Send socket for ~100-byte JSON telemetry frames.
        let tel_rx_buf = PacketBuffer::new(vec![PacketMetadata::EMPTY; 1], vec![0u8; 64]);
        let tel_tx_buf = PacketBuffer::new(vec![PacketMetadata::EMPTY; 4], vec![0u8; 512]);
        let tel_socket = UdpSocket::new(tel_rx_buf, tel_tx_buf);

        let mut sockets: SocketSet<'static> = SocketSet::new(vec![]);
        let cmd_handle = sockets.add(cmd_socket);
        let tel_handle = sockets.add(tel_socket);

        // Bind sockets to their respective ports.
        sockets
            .get_mut::<UdpSocket>(cmd_handle)
            .bind(WIFI_CMD_PORT)
            .map_err(|e| error!("WiFi: bind cmd socket error: {:?}", e))?;

        // Bind telemetry socket to a port so smoltcp accepts outbound traffic.
        sockets
            .get_mut::<UdpSocket>(tel_handle)
            .bind(WIFI_TEL_PORT)
            .map_err(|e| error!("WiFi: bind tel socket error: {:?}", e))?;

        info!(
            "WiFi: UDP cmd=:{} tel=:{} bcast={}.{}.{}.{}",
            WIFI_CMD_PORT, WIFI_TEL_PORT,
            WIFI_BROADCAST_IP[0], WIFI_BROADCAST_IP[1],
            WIFI_BROADCAST_IP[2], WIFI_BROADCAST_IP[3]
        );

        Ok(WifiAdapterInner {
            _controller: controller,
            iface,
            bridge,
            sockets,
            cmd_handle,
            tel_handle,
            pending_throttle: None,
            pending_button: false,
            last_tel_ms: 0,
        })
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Pump the smoltcp stack and drain the command socket.
    fn poll_stack(inner: &mut WifiAdapterInner, now_ms: u64) {
        let smoltcp_now = smoltcp::time::Instant::from_millis(now_ms as i64);

        // One smoltcp poll tick — processes pending Ethernet frames.
        inner
            .iface
            .poll(smoltcp_now, &mut inner.bridge, &mut inner.sockets);

        // Drain incoming command packets.
        let mut buf = [0u8; 64];
        loop {
            let sock = inner.sockets.get_mut::<UdpSocket>(inner.cmd_handle);
            if !sock.can_recv() {
                break;
            }
            match sock.recv_slice(&mut buf) {
                Ok((4, _endpoint)) if buf[0] == 0xA5 => match buf[1] {
                    0x01 => {
                        // Throttle — v1/v2 transmitted as u8, reinterpreted as i8.
                        let tl = buf[2] as i8;
                        let tr = buf[3] as i8;
                        trace!("WiFi CMD throttle L={:+} R={:+}", tl, tr);
                        inner.pending_throttle = Some((tl, tr));
                    }
                    0x02 => {
                        trace!("WiFi CMD button press");
                        inner.pending_button = true;
                    }
                    t => warn!("WiFi CMD: unknown type {:#04x}", t),
                },
                Ok((len, _)) => warn!("WiFi CMD: unexpected packet length {}", len),
                Err(_) => break,
            }
        }
    }

    /// Format and emit a telemetry JSON frame via UDP broadcast.
    fn send_telemetry(inner: &mut WifiAdapterInner, frame: &TelemetryFrame) {
        // Rate-limit: do not send more often than TELEMETRY_INTERVAL_MS.
        if frame.uptime_ms.saturating_sub(inner.last_tel_ms) < TELEMETRY_INTERVAL_MS {
            return;
        }

        // Build compact JSON in a stack-allocated 192-byte string.
        let mut json: HString<192> = HString::new();
        let ll = frame.lidar_l_cm.map_or(-1i32, |v| v as i32);
        let lr = frame.lidar_r_cm.map_or(-1i32, |v| v as i32);
        if write!(
            json,
            r#"{{"s":"{}","ll":{},"lr":{},"tl":{},"tr":{},"ms":{}}}"#,
            frame.state_name,
            ll,
            lr,
            frame.throttle_l,
            frame.throttle_r,
            frame.uptime_ms
        )
        .is_err()
        {
            warn!("WiFi TEL: JSON format overflow");
            return;
        }

        let [ba, bb, bc, bd] = WIFI_BROADCAST_IP;
        let dest: IpEndpoint = IpEndpoint {
            addr: IpAddress::Ipv4(Ipv4Address::new(ba, bb, bc, bd)),
            port: WIFI_TEL_PORT,
        };

        let sock = inner.sockets.get_mut::<UdpSocket>(inner.tel_handle);
        if sock.can_send() {
            match sock.send_slice(json.as_bytes(), dest) {
                Ok(()) => {
                    trace!("WiFi TEL: {}", json.as_str());
                    inner.last_tel_ms = frame.uptime_ms;
                }
                Err(e) => warn!("WiFi TEL: send_slice error: {:?}", e),
            }
        } else {
            warn!("WiFi TEL: send buffer full, frame dropped");
        }
    }
}

// ── Port trait implementations ────────────────────────────────────────────────

impl RemoteControlPort for WifiAdapter {
    /// Pump the network stack and parse incoming command packets.
    ///
    /// Must be called once per robot tick so that smoltcp drains the driver
    /// Rx queue and any pending command is buffered for `poll_throttle()` /
    /// `poll_button()`.
    fn poll_network(&mut self, now_ms: u64) {
        if let Some(inner) = &mut self.inner {
            Self::poll_stack(inner, now_ms);
        }
    }

    /// Return and consume the most-recent remote throttle command, if any.
    fn poll_throttle(&mut self) -> Option<(i8, i8)> {
        self.inner.as_mut().and_then(|i| i.pending_throttle.take())
    }

    /// Return and consume a pending remote button event.
    fn poll_button(&mut self) -> bool {
        self.inner
            .as_mut()
            .map(|i| {
                let pressed = i.pending_button;
                i.pending_button = false;
                pressed
            })
            .unwrap_or(false)
    }
}

impl TelemetryPort for WifiAdapter {
    /// Broadcast a telemetry JSON frame over UDP.
    ///
    /// Rate-limited internally to [`TELEMETRY_INTERVAL_MS`]; calling more
    /// frequently is harmless.
    ///
    /// [`TELEMETRY_INTERVAL_MS`]: crate::config::TELEMETRY_INTERVAL_MS
    fn send(&mut self, frame: &TelemetryFrame) {
        if let Some(inner) = &mut self.inner {
            Self::send_telemetry(inner, frame);
        }
    }
}
