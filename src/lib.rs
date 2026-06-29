mod abs_pointer;
#[path = "atspi_tree.rs"]
mod atspi_tree_impl;
mod cli;
mod cosmic_helper;
#[path = "diagnostics.rs"]
mod diagnostics_impl;
mod gnome_extension;
mod identity;
mod remote_desktop;
#[path = "screenshot.rs"]
mod screenshot_impl;
mod server;
mod terminal;
mod windowing;
mod windows;

pub mod atspi_tree {
    pub(crate) use crate::atspi_tree_impl::{
        list_accessible_apps, perform_action, set_element_value, AccessibleAppSummary,
        ValueSetInvocation,
    };
    pub use crate::atspi_tree_impl::{
        snapshot_tree, AccessibilityAction, AccessibilityNode, AccessibilityText,
        AccessibilityTextSelection, AccessibilityValue, Bounds,
    };
}

pub mod diagnostics {
    pub use crate::diagnostics_impl::{
        doctor_report, hydrate_session_bus_env, AccessibilityReport, CapabilityMap, Check,
        DoctorReport, InputReport, PlatformReport, PortalReport, PreferredBackends,
        ReadinessReport, WindowingReport,
    };
    pub(crate) use crate::diagnostics_impl::{setup_accessibility_report, SetupReport};
}

pub mod screenshot {
    pub(crate) use crate::screenshot_impl::{
        capture_screenshot, prepare_screenshot_payload, ScreenshotCapture, ScreenshotOutputFormat,
        ScreenshotPayloadOptions,
    };
    pub use crate::screenshot_impl::{capture_screenshot_raw, RawScreenshotCapture};
}

#[doc(hidden)]
pub async fn run_cli_from_env() -> anyhow::Result<()> {
    cli::run_from_env().await
}
