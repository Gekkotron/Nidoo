//! The Frigate cameras the hub can show live (`GET /api/v1/cameras`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The camera list and whether the hub serves live frames (`[frigate] live`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CamerasView {
    pub live: bool,
    pub cameras: Vec<CameraInfo>,
}

/// One Frigate camera: its raw id (`front_door`) and display name (`Front Door`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CameraInfo {
    pub id: String,
    pub name: String,
}
