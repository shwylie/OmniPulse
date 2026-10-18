// Seamless Mix: how two songs are changed over (crate::smart, the planner) and what the audio engine
// is given to do it (crate::plan). One crate for the desktop app and the Android app, so a mix is the
// same on both.

pub mod plan;
pub mod smart;
pub mod tape;
