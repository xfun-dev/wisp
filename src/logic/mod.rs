//! Platform-independent behavior, unit-tested on any host. The platform layers
//! only translate OS events in and carry side effects out; every decision lives here.

#[cfg(any(test, target_os = "macos"))]
pub mod capture;
#[cfg(any(test, feature = "bridge"))]
pub mod bridge;
pub mod clipboard;
pub mod pairing;
#[cfg(any(test, windows))]
pub mod icon;
#[cfg(any(test, windows))]
pub mod press;
#[cfg(any(test, windows))]
pub mod replay;
#[cfg(any(test, windows))]
pub mod service;
#[cfg(any(test, target_os = "macos"))]
pub mod settings;
#[cfg(any(test, windows))]
pub mod status;
