//! A house satellite without the hardware: what a desktop shell (the Mac menu-bar app)
//! needs to be a Sendspin player and a voice satellite, with no audio device, microphone
//! or UI of its own. Everything here is platform-neutral and tested on Linux.

pub mod brain;
pub mod playout;
pub mod run;
pub mod sendspin;
pub mod store;
pub mod voice;
pub mod wake_head;

pub use brain::{IconState, Trainer, UiState, WakeDetector};
pub use playout::SharedPlayout;
pub use run::{Config, Control, Handles, MicSwitch, start};
