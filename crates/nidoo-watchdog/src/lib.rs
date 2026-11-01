//! Gateway health and broker ACL watchdogs: pure translators ported from OikosPush's
//! `gateway_health_watchdog.py` and `broker_acl_watchdog.py`.
//!
//! Each watchdog takes the clock as an argument and returns the v1 push alert as JSON; the
//! caller hands it to the push service (`source` `system`). [`tail`] feeds the ACL watchdog
//! from a Mosquitto log file.
#![forbid(unsafe_code)]

mod acl;
mod gateway;
mod tail;

pub use acl::AclWatchdog;
pub use gateway::GatewayWatchdog;
pub use tail::tail;

/// Whether the MQTT subscription `filter` covers `topic` (`+` and `#`; a leading wildcard
/// never covers a `$` topic).
pub fn topic_matches(filter: &str, topic: &str) -> bool {
    if topic.starts_with('$') && (filter.starts_with('+') || filter.starts_with('#')) {
        return false;
    }
    let mut filter = filter.split('/');
    let mut topic = topic.split('/');
    loop {
        match (filter.next(), topic.next()) {
            (Some("#"), _) | (None, None) => return true,
            (Some("+"), Some(_)) => {}
            (Some(f), Some(t)) if f == t => {}
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::topic_matches;

    #[test]
    fn filters_match_like_mqtt() {
        assert!(topic_matches("a/+", "a/b"));
        assert!(!topic_matches("a/+", "a/b/c"));
        assert!(topic_matches("$SYS/broker/log/#", "$SYS/broker/log/N"));
        assert!(topic_matches("$SYS/broker/log/#", "$SYS/broker/log"));
        assert!(!topic_matches("#", "$SYS/broker/log/N"));
        assert!(!topic_matches("a/b", "a"));
    }
}
