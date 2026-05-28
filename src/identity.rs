//! Build-time identity values for the optional GNOME Shell window-control bridge.
//!
//! The standalone crate keeps the public `computer-use-linux` DBus/extension
//! identity by default. Downstream bundles can compile with `CUL_*` env vars to
//! brand the same code under their own GNOME extension UUID and DBus endpoint
//! without carrying source-only string patches.

pub const DEFAULT_GNOME_EXTENSION_UUID: &str = "computer-use-linux@avifenesh.dev";
pub const DEFAULT_DBUS_SERVICE: &str = "dev.avifenesh.ComputerUseLinux.WindowControl";
pub const DEFAULT_DBUS_OBJECT_PATH: &str = "/dev/avifenesh/ComputerUseLinux/WindowControl";

pub const GNOME_EXTENSION_UUID: &str = match option_env!("CUL_GNOME_EXTENSION_UUID") {
    Some(value) => value,
    None => DEFAULT_GNOME_EXTENSION_UUID,
};

pub const DBUS_SERVICE: &str = match option_env!("CUL_DBUS_SERVICE") {
    Some(value) => value,
    None => DEFAULT_DBUS_SERVICE,
};

pub const DBUS_OBJECT_PATH: &str = match option_env!("CUL_DBUS_OBJECT_PATH") {
    Some(value) => value,
    None => DEFAULT_DBUS_OBJECT_PATH,
};
