//! `RobotId` — strongly-typed robot identity (its IPv4 address string).

use std::fmt;

/// Strongly-typed wrapper around a robot's IP address string.
///
/// Kept as a `String` internally so it can be stored in PostgreSQL without
/// conversion, forwarded in JSON, and used as a HashMap key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RobotId(String);

impl RobotId {
    /// Create a new `RobotId`.  The value is stored as-is; the constructor
    /// accepts any non-empty string (IPv4 addresses are the convention).
    pub fn new(ip: impl Into<String>) -> Self {
        Self(ip.into())
    }

    /// Borrow the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RobotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for RobotId {
    fn from(s: String) -> Self { Self(s) }
}

impl From<&str> for RobotId {
    fn from(s: &str) -> Self { Self(s.to_owned()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_returns_inner_string() {
        let id = RobotId::new("192.168.1.42");
        assert_eq!(id.to_string(), "192.168.1.42");
    }

    #[test]
    fn equality_by_value() {
        assert_eq!(RobotId::new("10.0.0.1"), RobotId::new("10.0.0.1"));
        assert_ne!(RobotId::new("10.0.0.1"), RobotId::new("10.0.0.2"));
    }
}
