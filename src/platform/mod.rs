#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{configure_shortcuts, run};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{configure_shortcuts, run};
