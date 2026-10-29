use anyhow::{bail, Context, Result};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub(crate) fn peer_pid(fd: RawFd) -> Result<u32> {
    // SAFETY: zero is a valid initial value for the plain credentials structure.
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials and size are writable objects of the declared size.
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::from_mut(&mut credentials).cast(),
            &mut size,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .context("could not identify compositor socket peer");
    }
    // SAFETY: geteuid has no preconditions or memory effects.
    if credentials.pid <= 0 || credentials.uid != unsafe { libc::geteuid() } {
        bail!("compositor socket is not owned by the desktop user");
    }
    Ok(credentials.pid as u32)
}

pub(crate) fn compositor_pid() -> Option<u32> {
    if let Ok(fd) = std::env::var("WAYLAND_SOCKET") {
        return peer_pid(fd.parse().ok()?).ok();
    }
    peer_pid_at(&display_path()?)
}

pub(crate) fn display_path() -> Option<PathBuf> {
    display_path_from(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
    )
}

fn display_path_from(
    display: Option<&std::ffi::OsStr>,
    runtime: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let display = display.filter(|value| !value.is_empty())?;
    let display = Path::new(display);
    if display.is_absolute() {
        Some(display.to_path_buf())
    } else {
        Some(Path::new(runtime?).join(display))
    }
}

pub(crate) fn peer_pid_at(path: &Path) -> Option<u32> {
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: zero initializes an empty Unix socket address.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return None;
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    // Nonblocking connect makes a full compositor backlog a failed diagnostic,
    // rather than letting a readiness query stall the server.
    // SAFETY: socket has no pointer arguments and returns a new owned fd.
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if raw < 0 {
        return None;
    }
    // SAFETY: raw is a newly created descriptor owned by this function.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: address contains a terminated path and remains valid for connect.
    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            std::ptr::from_ref(&address).cast(),
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    };
    if result != 0 {
        return None;
    }
    peer_pid(fd.as_raw_fd()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_selection_requires_an_explicit_display() {
        use std::ffi::OsStr;
        assert!(display_path_from(None, Some(OsStr::new("/run/user/1000"))).is_none());
        assert!(
            display_path_from(Some(OsStr::new("")), Some(OsStr::new("/run/user/1000"))).is_none()
        );
        assert_eq!(
            display_path_from(
                Some(OsStr::new("nested")),
                Some(OsStr::new("/run/user/1000"))
            ),
            Some(PathBuf::from("/run/user/1000/nested"))
        );
    }

    #[test]
    fn identifies_the_connected_peer_without_environment_guessing() {
        let (client, _server) = std::os::unix::net::UnixStream::pair().unwrap();
        assert_eq!(peer_pid(client.as_raw_fd()).unwrap(), std::process::id());
        assert!(peer_pid(-1).is_err());
    }
}
