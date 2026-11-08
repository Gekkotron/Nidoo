//! Wire types for the phone setup code: what the hub shows and takes for the Android app's QR.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::setup::{Secret, SecretView};

/// The mobile section as a read shows it: the gateway key is never sent back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MobileView {
    pub gateway_url: Option<String>,
    pub gateway_key: SecretView,
    /// Both the gateway address and its key are set, so the code carries a gateway.
    pub gateway_ready: bool,
}

/// The mobile section in a `PUT`. A `PUT` replaces the whole section: a left-out address is
/// cleared. Only the key follows `Secret`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MobileInput {
    #[serde(default)]
    pub gateway_url: Option<String>,
    #[serde(default)]
    pub gateway_key: Secret,
}

/// The text the setup QR code encodes. It holds the broker password, the gateway key and the
/// Frigate password, so `Debug` never prints it.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MobileCode {
    pub payload: String,
}

impl std::fmt::Debug for MobileCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MobileCode")
            .field("payload", &"***")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_masks_the_payload() {
        let code = MobileCode {
            payload: r#"{"v":1,"local":{"pass":"secret-pass"}}"#.into(),
        };
        assert!(!format!("{code:?}").contains("secret-pass"));
    }
}
