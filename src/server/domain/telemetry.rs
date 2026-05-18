//! `TelemetryFrame` — domain value object decoded from a robot UDP frame.
//!
//! Decoded from either:
//! - Protobuf binary (primary, `proto::TelemetryFrame`)
//! - Legacy JSON (backwards compatibility; `{"s":"IDLE","ll":-1,...}`)

use super::RobotId;

/// A single telemetry snapshot emitted by a robot.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TelemetryFrame {
    /// Robot FSM state name: `IDLE` | `RECORD` | `READY` | `PLAY` | `AVOIDING` | `HALT`.
    pub state: String,
    /// Left LIDAR distance in cm; `None` when sensor is stale or absent.
    pub lidar_left_cm: Option<u16>,
    /// Right LIDAR distance in cm; `None` when sensor is stale or absent.
    pub lidar_right_cm: Option<u16>,
    /// Left-motor throttle `[-100, 100]`.
    pub throttle_left: i8,
    /// Right-motor throttle `[-100, 100]`.
    pub throttle_right: i8,
    /// Milliseconds since robot boot.
    pub uptime_ms: u64,
    /// Robot's identity (DHCP-assigned IP, embedded by the robot itself).
    pub robot_id: RobotId,
}

impl TelemetryFrame {
    /// Decode from a protobuf-encoded UDP payload.
    pub fn from_proto(raw: &[u8]) -> Result<Self, prost::DecodeError> {
        use prost::Message as _;
        let proto = crate::proto::TelemetryFrame::decode(raw)?;
        Ok(Self::from(proto))
    }

    /// Decode from a legacy JSON payload (backwards compatibility).
    pub fn from_json(raw: &[u8], fallback_ip: &str) -> Result<Self, serde_json::Error> {
        #[derive(serde::Deserialize)]
        struct JsonFrame {
            #[serde(rename = "s")]
            state: String,
            #[serde(rename = "ll")]
            lidar_left_cm: i32,
            #[serde(rename = "lr")]
            lidar_right_cm: i32,
            #[serde(rename = "tl")]
            throttle_left: i8,
            #[serde(rename = "tr")]
            throttle_right: i8,
            #[serde(rename = "ms")]
            uptime_ms: u64,
            #[serde(rename = "ip")]
            robot_ip: Option<String>,
        }

        let j: JsonFrame = serde_json::from_slice(raw)?;
        let ip = j.robot_ip.unwrap_or_else(|| fallback_ip.to_owned());
        Ok(Self {
            state: j.state,
            lidar_left_cm: if j.lidar_left_cm < 0 { None } else { Some(j.lidar_left_cm as u16) },
            lidar_right_cm: if j.lidar_right_cm < 0 { None } else { Some(j.lidar_right_cm as u16) },
            throttle_left: j.throttle_left,
            throttle_right: j.throttle_right,
            uptime_ms: j.uptime_ms,
            robot_id: RobotId::new(ip),
        })
    }

    /// Parse a raw UDP payload — tries protobuf first, then JSON fallback.
    ///
    /// The first byte distinguishes the two formats:
    /// - `{` (0x7B) → JSON (legacy)
    /// - anything else → protobuf
    pub fn decode(raw: &[u8], fallback_ip: &str) -> Result<Self, DecodeError> {
        if raw.first() == Some(&b'{') {
            Self::from_json(raw, fallback_ip).map_err(DecodeError::Json)
        } else {
            Self::from_proto(raw).map_err(DecodeError::Proto)
        }
    }
}

/// Error variants for `TelemetryFrame::decode`.
#[derive(Debug)]
pub enum DecodeError {
    Proto(prost::DecodeError),
    Json(serde_json::Error),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Proto(e) => write!(f, "protobuf decode error: {e}"),
            Self::Json(e)  => write!(f, "JSON decode error: {e}"),
        }
    }
}

impl From<crate::proto::TelemetryFrame> for TelemetryFrame {
    fn from(p: crate::proto::TelemetryFrame) -> Self {
        Self {
            state: p.state,
            lidar_left_cm:  if p.lidar_left_cm  < 0 { None } else { Some(p.lidar_left_cm  as u16) },
            lidar_right_cm: if p.lidar_right_cm < 0 { None } else { Some(p.lidar_right_cm as u16) },
            throttle_left:  p.throttle_left  as i8,
            throttle_right: p.throttle_right as i8,
            uptime_ms:      p.uptime_ms,
            robot_id:       RobotId::new(p.robot_ip),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;

    fn sample_proto() -> crate::proto::TelemetryFrame {
        crate::proto::TelemetryFrame {
            state:           "PLAY".into(),
            lidar_left_cm:   125,
            lidar_right_cm:  -1,
            throttle_left:   50,
            throttle_right:  50,
            uptime_ms:       12_345,
            robot_ip:        "192.168.1.42".into(),
        }
    }

    #[test]
    fn round_trip_proto() {
        let encoded = sample_proto().encode_to_vec();
        let frame = TelemetryFrame::from_proto(&encoded).unwrap();
        assert_eq!(frame.state, "PLAY");
        assert_eq!(frame.lidar_left_cm, Some(125));
        assert_eq!(frame.lidar_right_cm, None);   // -1 → None
        assert_eq!(frame.throttle_left, 50);
        assert_eq!(frame.uptime_ms, 12_345);
        assert_eq!(frame.robot_id.as_str(), "192.168.1.42");
    }

    #[test]
    fn decode_dispatches_to_json_on_brace() {
        let json = br#"{"s":"IDLE","ll":-1,"lr":-1,"tl":0,"tr":0,"ms":0,"ip":"10.0.0.1"}"#;
        let frame = TelemetryFrame::decode(json, "fallback").unwrap();
        assert_eq!(frame.state, "IDLE");
        assert_eq!(frame.robot_id.as_str(), "10.0.0.1");
    }

    #[test]
    fn decode_dispatches_to_proto_on_non_brace() {
        let encoded = sample_proto().encode_to_vec();
        // First byte of a protobuf varint is never `{` (0x7B = 123).
        // Field 1 (string state "PLAY") encodes as 0x0A ...
        assert_ne!(encoded[0], b'{');
        let frame = TelemetryFrame::decode(&encoded, "fallback").unwrap();
        assert_eq!(frame.state, "PLAY");
    }

    #[test]
    fn json_fallback_ip_used_when_no_ip_field() {
        let json = br#"{"s":"IDLE","ll":-1,"lr":-1,"tl":0,"tr":0,"ms":0}"#;
        let frame = TelemetryFrame::decode(json, "1.2.3.4").unwrap();
        assert_eq!(frame.robot_id.as_str(), "1.2.3.4");
    }

    #[test]
    fn lidar_absent_when_negative() {
        let json = br#"{"s":"IDLE","ll":-1,"lr":50,"tl":0,"tr":0,"ms":0}"#;
        let frame = TelemetryFrame::from_json(json, "x").unwrap();
        assert_eq!(frame.lidar_left_cm, None);
        assert_eq!(frame.lidar_right_cm, Some(50));
    }
}
