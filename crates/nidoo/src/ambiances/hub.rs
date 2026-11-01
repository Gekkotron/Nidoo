//! The ambiance side of the hub: the read views and the writers behind the REST layer.

use nidoo_api::{Ambiance, AmbianceAction, AmbianceView, WsEvent, validate_ambiance};
use nidoo_core::command::{Publish, map_command};
use nidoo_core::model::{Command, UNASSIGNED_AREA};

use super::{StoreApplied, override_request};
use crate::hub::{Hub, HubError};

impl Hub {
    /// Every stored ambiance, valid first (by order), then the invalid ones.
    pub fn ambiances(&self) -> Vec<AmbianceView> {
        self.ambiance_store().list()
    }

    pub fn ambiance(&self, id: &str) -> Option<AmbianceView> {
        self.ambiance_store().view(id)
    }

    /// Create-or-replace: validates the shape and the house, publishes retained and applies
    /// locally so the next read sees it before the broker echo arrives.
    pub async fn set_ambiance(&self, mut a: Ambiance) -> Result<AmbianceView, HubError> {
        validate_ambiance(&a).map_err(HubError::Rejected)?;
        let stored = self.ambiance(&a.id).and_then(|view| view.ambiance);
        self.check_ambiance_house(&mut a, stored.as_ref())?;
        let payload = serde_json::to_string(&a)
            .map_err(|err| HubError::Rejected(format!("serialize: {err}")))?;
        self.publish(Publish {
            topic: self.ambiance_topic(&a.id),
            payload: payload.clone(),
            retain: true,
        })?;
        self.ingest_ambiance(&a.id, payload.as_bytes());
        Ok(AmbianceView {
            id: a.id.clone(),
            ambiance: Some(a),
            invalid: None,
        })
    }

    /// Removes an ambiance by publishing an empty retained payload and applying it locally.
    pub async fn delete_ambiance(&self, id: &str) -> Result<(), HubError> {
        if !self.ambiance_store().knows(id) {
            return Err(HubError::UnknownAmbiance(id.to_owned()));
        }
        self.publish(Publish {
            topic: self.ambiance_topic(id),
            payload: String::new(),
            retain: true,
        })?;
        self.ingest_ambiance(id, b"");
        Ok(())
    }

    /// Applies a retained payload and tells the clients when the list changed.
    pub(crate) fn ingest_ambiance(&self, id: &str, payload: &[u8]) {
        let applied = self.ambiance_store().apply(id, payload);
        if matches!(applied, StoreApplied::Changed | StoreApplied::Removed) {
            self.broadcast(WsEvent::AmbiancesChanged {
                ambiances: self.ambiances(),
            });
        }
    }

    /// Checks every action against the house; a category's area is stored as its resolved id.
    /// An action equal to one of the stored version is not checked again, so an ambiance whose
    /// entity, area, flow or zone has since gone can still be renamed or reordered. Publishes
    /// nothing.
    fn check_ambiance_house(
        &self,
        a: &mut Ambiance,
        stored: Option<&Ambiance>,
    ) -> Result<(), HubError> {
        for (index, action) in a.actions.iter_mut().enumerate() {
            if stored.is_some_and(|s| s.actions.contains(action)) {
                continue;
            }
            let reject = |why: String| HubError::Rejected(format!("action {index}: {why}"));
            match action {
                AmbianceAction::Device(device) => {
                    let reg = self.read();
                    let Some(found) = reg.entity(&device.entity) else {
                        return Err(reject(format!("unknown entity `{}`", device.entity)));
                    };
                    if matches!(
                        device.command,
                        Command::Arm { code: Some(_), .. } | Command::Disarm { code: Some(_) }
                    ) {
                        return Err(reject(
                            "alarm codes are not allowed in ambiances (they would be stored in clear on the broker)"
                                .to_owned(),
                        ));
                    }
                    let state = reg.state(&device.entity).cloned().unwrap_or_default();
                    map_command(found, &state, &device.command, reg.templates())
                        .map_err(|err| reject(err.to_string()))?;
                }
                AmbianceAction::Category(category) => {
                    let reg = self.read();
                    if let Some(raw) = category.area.clone() {
                        let resolved = reg.resolve_area(&raw);
                        let known = reg
                            .effective_areas()
                            .iter()
                            .any(|area| area.id == resolved && area.id != UNASSIGNED_AREA);
                        if !known {
                            return Err(reject(format!("unknown area `{raw}`")));
                        }
                        category.area = Some(resolved);
                    }
                    if let Some(missing) =
                        category.except.iter().find(|id| reg.entity(id).is_none())
                    {
                        return Err(reject(format!("unknown entity `{missing}` in except")));
                    }
                }
                AmbianceAction::Flow(flow) => {
                    if !self.flow_store().exists(&flow.flow) {
                        return Err(reject(format!("unknown flow `{}`", flow.flow)));
                    }
                }
                AmbianceAction::Climate(climate) => {
                    if !self.climate_enabled() {
                        return Err(reject("the climate engine is off".to_owned()));
                    }
                    if let Some(zone) = &climate.zone
                        && !self.climate_zone_ids().iter().any(|z| z == zone)
                    {
                        return Err(reject(format!("unknown zone `{zone}`")));
                    }
                    if !climate.clear {
                        override_request(climate).map_err(|err| reject(err.to_string()))?;
                    }
                }
            }
        }
        Ok(())
    }
}
