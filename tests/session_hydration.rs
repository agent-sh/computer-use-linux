use computer_use_linux::diagnostics::hydrate_session_bus_env;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

#[test]
fn session_discovery_is_reused_and_environment_changes_refresh_it() {
    if std::env::var_os("CUL_HYDRATION_TEST_CHILD").is_some() {
        let log = PathBuf::from(std::env::var_os("CUL_HYDRATION_TEST_LOG").unwrap());
        for _ in 0..3 {
            hydrate_session_bus_env();
        }
        assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 1);
        std::env::set_var("DISPLAY", ":45679");
        hydrate_session_bus_env();
        assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 2);
        let fail = PathBuf::from(std::env::var_os("CUL_HYDRATION_TEST_FAIL").unwrap());
        fs::write(&fail, "fail").unwrap();
        std::env::remove_var("XDG_CURRENT_DESKTOP");
        std::env::remove_var("XDG_SESSION_TYPE");
        hydrate_session_bus_env();
        fs::remove_file(fail).unwrap();
        hydrate_session_bus_env();
        hydrate_session_bus_env();
        assert_eq!(fs::read_to_string(log).unwrap().lines().count(), 4);
        assert!(!std::env::var("XDG_CURRENT_DESKTOP").unwrap().is_empty());
        return;
    }

    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache"));
    let root = cache.join(format!("cul-hydration-test-{}", getrandom::u64().unwrap()));
    fs::create_dir_all(&root).unwrap();
    let fixture = root.join("systemctl");
    fs::write(
        &fixture,
        "#!/bin/sh\nprintf 'discovery\\n' >> \"$CUL_HYDRATION_TEST_LOG\"\nif test -f \"$CUL_HYDRATION_TEST_FAIL\"; then exit 1; fi\nprintf 'XDG_CURRENT_DESKTOP=Fixture\\nXDG_SESSION_TYPE=x11\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&fixture, fs::Permissions::from_mode(0o700)).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session_discovery_is_reused_and_environment_changes_refresh_it",
            "--test-threads=1",
        ])
        .env("CUL_HYDRATION_TEST_CHILD", "1")
        .env("CUL_HYDRATION_TEST_LOG", root.join("discovery.log"))
        .env("CUL_HYDRATION_TEST_FAIL", root.join("fail"))
        .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent-cul-test-bus",
        )
        .env("XDG_RUNTIME_DIR", &root)
        .env("XDG_CURRENT_DESKTOP", "Fixture")
        .env("XDG_SESSION_TYPE", "x11")
        .env("DISPLAY", ":45678")
        .env_remove("WAYLAND_DISPLAY")
        .status()
        .unwrap();
    fs::remove_dir_all(root).unwrap();
    assert!(status.success());
}
