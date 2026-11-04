//! Speech input: satellites stream their microphone to the hub after a wake word, the hub
//! finds the end of the phrase, transcribes it and answers through the assistant.

pub mod guard;
pub mod packet;
