//! Paths for unit tests that bind or connect to Unix sockets, and the
//! private D-Bus daemon they share.
//!
//! A socket path must fit `sun_path`, so a fixture under a long `TMPDIR`
//! fails with a bind error that reads like a product bug. Fixtures pick their
//! place here instead: the temp dir when the longest socket path fits, else
//! `XDG_RUNTIME_DIR`, else a panic that names the limit and the path.
//! `scripts/socket_fixture.py` applies the same rule to the Python fixtures.
//! `tests/window_targeting_setup.rs` includes this file too.

use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

/// `sun_path` holds 108 bytes, including the terminating NUL.
const SOCKET_PATH_MAX: usize = 107;
/// dbus-daemon refuses a listen path longer than this.
const DBUS_SOCKET_PATH_MAX: usize = 99;
/// The socket dbus-daemon makes for `<listen>unix:dir=DIR</listen>`.
const DBUS_DIR_SOCKET: &str = "dbus-XXXXXXXXXX";

/// A dbus-daemon whose socket stays in a fixture directory of its own.
///
/// The config has no service directories, so nothing on the bus can
/// activate a desktop service, and the daemon starts with an empty
/// environment. dbus-daemon leaves its socket behind when it is killed, so
/// drop removes the directory after the daemon exits.
pub(crate) struct PrivateBus {
    child: Child,
    dir: PathBuf,
    address: String,
}

impl PrivateBus {
    /// Starts a bus in a new fixture directory whose name begins with `label`.
    pub(crate) fn start(label: &str) -> Self {
        let dir = bus_fixture(
            &format!("{label}-{:016x}", getrandom::u64().unwrap()),
            DBUS_DIR_SOCKET,
        );
        fs::create_dir_all(&dir).expect("create private bus dir");
        // The directory also serves as a private XDG_RUNTIME_DIR.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let config = dir.join("bus.conf");
        fs::write(
            &config,
            format!(
                "<busconfig><type>session</type><listen>unix:dir={}</listen>\
                 <policy context=\"default\"><allow own=\"*\"/>\
                 <allow send_destination=\"*\"/><allow receive_sender=\"*\"/>\
                 </policy></busconfig>",
                dbus_address_value(&dir)
            ),
        )
        .unwrap();
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--nopidfile", "--print-address=1"])
            .env_clear()
            .stdout(Stdio::piped())
            .spawn()
            .expect("start private dbus-daemon");
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .expect("read private bus address");
        let bus = Self {
            child,
            dir,
            address: address.trim().to_string(),
        };
        assert!(!bus.address.is_empty(), "private bus printed no address");
        bus
    }

    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// The fixture directory, which holds the socket and the config.
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// `path` as a D-Bus address value: bytes outside `[-0-9A-Za-z_/.\*]` are
/// written as `%XX`. The result needs no XML escaping either.
fn dbus_address_value(path: &Path) -> String {
    path.as_os_str()
        .as_bytes()
        .iter()
        .map(|&byte| {
            if byte.is_ascii_alphanumeric() || b"-_/.\\*".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02x}")
            }
        })
        .collect()
}

/// Path of the fixture `name`, whose longest socket path is `name/deepest`,
/// or `name` itself when `deepest` is empty. Nothing is created.
pub(crate) fn socket_fixture(name: &str, deepest: &str) -> PathBuf {
    fixture(name, deepest, SOCKET_PATH_MAX)
}

/// [`socket_fixture`] for a directory that holds a private dbus-daemon's
/// listen socket, `name/socket`.
pub(crate) fn bus_fixture(name: &str, socket: &str) -> PathBuf {
    fixture(name, socket, DBUS_SOCKET_PATH_MAX)
}

fn fixture(name: &str, deepest: &str, limit: usize) -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from);
    place(
        name,
        deepest,
        limit,
        &std::env::temp_dir(),
        runtime.as_deref(),
    )
    .unwrap_or_else(|message| panic!("{message}"))
}

fn place(
    name: &str,
    deepest: &str,
    limit: usize,
    temp: &Path,
    runtime: Option<&Path>,
) -> Result<PathBuf, String> {
    let socket = |base: &Path| {
        let path = base.join(name);
        if deepest.is_empty() {
            path
        } else {
            path.join(deepest)
        }
    };
    let overflow = |base: &Path| {
        let path = socket(base);
        let length = path.as_os_str().len();
        (length > limit).then(|| {
            format!(
                "{} is {length} bytes, over the {limit}-byte limit",
                path.display()
            )
        })
    };
    let Some(first) = overflow(temp) else {
        return Ok(temp.join(name));
    };
    let fallback = match runtime {
        Some(dir) if dir.is_absolute() && dir.is_dir() => match overflow(dir) {
            None => return Ok(dir.join(name)),
            Some(second) => format!("under XDG_RUNTIME_DIR, {second}"),
        },
        Some(dir) => format!(
            "XDG_RUNTIME_DIR ({}) is not an absolute directory",
            dir.display()
        ),
        None => "XDG_RUNTIME_DIR is unset".to_string(),
    };
    Err(format!(
        "Unix socket path too long: {first}; {fallback}. \
         Set TMPDIR or XDG_RUNTIME_DIR to a shorter path."
    ))
}

mod tests {
    use super::*;

    #[test]
    fn a_long_temp_dir_moves_to_the_runtime_dir_or_names_the_limit() {
        let root = Path::new("/");
        // 96 bytes: a socket two short names below it fits 107 bytes but
        // not dbus-daemon's 99.
        let long = PathBuf::from(format!("/{}", "t".repeat(95)));
        assert_eq!(
            place("f", "bus", 107, &long, Some(root)),
            Ok(long.join("f"))
        );
        assert_eq!(place("f", "bus", 99, &long, Some(root)), Ok(root.join("f")));
        assert_eq!(
            place("fixture", "a.sock", 107, &long, Some(root)),
            Ok(root.join("fixture"))
        );

        let error = place("fixture", "a.sock", 107, &long, None).unwrap_err();
        let path = long.join("fixture/a.sock");
        assert!(
            error.contains(&format!(
                "{} is 111 bytes, over the 107-byte limit;",
                path.display()
            )),
            "{error}"
        );
        assert!(error.contains("XDG_RUNTIME_DIR is unset"), "{error}");
        let error = place("fixture", "a.sock", 10, &long, Some(root)).unwrap_err();
        assert!(
            error.contains("under XDG_RUNTIME_DIR, /fixture/a.sock is 15 bytes"),
            "{error}"
        );
        let error = place("f.sock", "", 10, &long, Some(Path::new("run"))).unwrap_err();
        assert!(error.contains("/f.sock is 103 bytes"), "{error}");
        assert!(
            error.contains("(run) is not an absolute directory"),
            "{error}"
        );
    }

    #[test]
    fn a_private_bus_keeps_its_socket_in_its_dir_and_removes_both() {
        let bus = PrivateBus::start("cul-bus-test");
        let dir = bus.dir().to_path_buf();
        let socket = PathBuf::from(
            bus.address()
                .strip_prefix("unix:path=")
                .and_then(|rest| rest.split(',').next())
                .unwrap_or_else(|| panic!("unexpected address {}", bus.address())),
        );
        assert_eq!(socket.parent(), Some(dir.as_path()));
        assert!(socket.exists());
        drop(bus);
        assert!(!socket.exists(), "{} outlived its bus", socket.display());
        assert!(!dir.exists(), "{} outlived its bus", dir.display());
    }

    #[test]
    fn bus_addresses_escape_bytes_outside_the_plain_set() {
        assert_eq!(dbus_address_value(Path::new("/a-b_c.d/e*")), "/a-b_c.d/e*");
        assert_eq!(dbus_address_value(Path::new("/a b/&%")), "/a%20b/%26%25");
    }

    /// Every private bus a test or script starts listens in a fixture
    /// directory: Rust tests use [`PrivateBus`] or pass their own config,
    /// scripts pass one from `socket_fixture.private_bus_config`. The session
    /// default and `unix:tmpdir` put the socket in `/tmp`, where a killed
    /// daemon leaves it behind.
    #[test]
    fn every_test_bus_listens_in_its_fixture() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let tmpdir_listen = concat!("unix:", "tmp", "dir=");
        let run_session = concat!("dbus-", "run-session");
        let daemon_spawn = concat!("Command::new(\"dbus-", "daemon\")");
        // All of src, and the files directly in tests and scripts.
        let mut files = Vec::new();
        let mut dirs = vec![root.join("src")];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        for dir in ["tests", "scripts"] {
            for entry in fs::read_dir(root.join(dir)).unwrap() {
                files.push(entry.unwrap().path());
            }
        }
        files.retain(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("rs" | "py" | "sh")
            )
        });
        let mut offenders = Vec::new();
        for path in files {
            let text = fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                // A call may put its config on one of the next two lines.
                let call = || lines[index..lines.len().min(index + 3)].join(" ");
                let starts_bus = line.contains(&format!("'{run_session}'"))
                    || line.contains(&format!("\"{run_session}\""))
                    || line.trim_start().starts_with(&format!("{run_session} "))
                    || line.contains(daemon_spawn);
                if line.contains(tmpdir_listen) || (starts_bus && !call().contains("--config-file"))
                {
                    offenders.push(format!("{}:{}", path.display(), index + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "private buses that listen outside their fixture: {offenders:?}"
        );
    }
}
