//! Service status: what the hub depends on, and since when it has been in its state.

use nidoo_api::{ServiceState, ServiceView, SystemService};
use nidoo_core::model::Availability;

/// The Mosquitto topic that counts the retained messages. The broker publishes it, not
/// retained, about every ten seconds.
pub const RETAINED_TOPIC: &str = "$SYS/broker/retained messages/count";

/// One service's current state and the time it last changed.
#[derive(Debug)]
pub struct ServiceStatus {
    state: ServiceState,
    since: Option<u64>,
}

impl Default for ServiceStatus {
    fn default() -> Self {
        Self {
            state: ServiceState::Unknown,
            since: None,
        }
    }
}

impl ServiceStatus {
    /// Record an observation at `now`. `since` moves only when the state changes.
    pub fn observe(&mut self, state: ServiceState, now: u64) {
        if self.since.is_none() || self.state != state {
            self.state = state;
            self.since = Some(now);
        }
    }

    /// The current state.
    pub fn state(&self) -> ServiceState {
        self.state
    }

    pub fn view(&self, service: SystemService) -> ServiceView {
        ServiceView {
            service,
            state: self.state,
            since: self.since,
        }
    }
}

/// The broker is up while the hub's MQTT connection is. Before the link was ever online or
/// lost (`link_epoch` 0, still connecting at boot) nothing is known yet: unknown, not down.
pub fn broker_state(online: bool, link_epoch: u64) -> ServiceState {
    if online {
        ServiceState::Up
    } else if link_epoch == 0 {
        ServiceState::Unknown
    } else {
        ServiceState::Down
    }
}

/// Zigbee2MQTT's bridge availability; never seen is unknown, not down.
pub fn bridge_state(availability: Availability) -> ServiceState {
    match availability {
        Availability::Online => ServiceState::Up,
        Availability::Offline => ServiceState::Down,
        Availability::Unknown => ServiceState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_since_changes_only_on_transition() {
        let mut status = ServiceStatus::default();
        assert_eq!(
            status.view(SystemService::Broker).state,
            ServiceState::Unknown
        );
        assert_eq!(status.view(SystemService::Broker).since, None);
        status.observe(ServiceState::Up, 10);
        status.observe(ServiceState::Up, 70);
        let view = status.view(SystemService::Broker);
        assert_eq!((view.state, view.since), (ServiceState::Up, Some(10)));
        status.observe(ServiceState::Down, 130);
        let view = status.view(SystemService::Broker);
        assert_eq!((view.state, view.since), (ServiceState::Down, Some(130)));
    }

    #[test]
    fn first_unknown_observation_stamps_since() {
        let mut status = ServiceStatus::default();
        status.observe(ServiceState::Unknown, 5);
        assert_eq!(status.view(SystemService::Database).since, Some(5));
    }

    #[test]
    fn bridge_availability_maps_to_service_state() {
        assert_eq!(bridge_state(Availability::Online), ServiceState::Up);
        assert_eq!(bridge_state(Availability::Offline), ServiceState::Down);
        assert_eq!(bridge_state(Availability::Unknown), ServiceState::Unknown);
        assert_eq!(broker_state(true, 0), ServiceState::Up);
        assert_eq!(broker_state(true, 3), ServiceState::Up);
        assert_eq!(broker_state(false, 1), ServiceState::Down);
    }

    #[test]
    fn broker_never_answered_since_boot_is_unknown() {
        // The first tick can run before the ConnAck: no answer yet is not an outage.
        assert_eq!(broker_state(false, 0), ServiceState::Unknown);
    }
}
