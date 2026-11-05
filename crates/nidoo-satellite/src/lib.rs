//! A house satellite without the hardware: what a desktop shell (the Mac menu-bar app)
//! needs to be a Sendspin player and a voice satellite, with no audio device, microphone
//! or UI of its own. Everything here is platform-neutral and tested on Linux.

pub mod playout;
pub mod sendspin;
pub mod store;
pub mod voice;
