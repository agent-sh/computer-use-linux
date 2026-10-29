// SPDX-License-Identifier: MIT

#[test]
fn gnome_indicator_capture_regressions() {
    let status = std::process::Command::new("/usr/bin/python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/gnome_indicator_test.py"
        ))
        .arg(env!("CARGO_BIN_EXE_computer-use-linux"))
        .arg(env!("CARGO_BIN_EXE_computer-use-linux-indicator"))
        .status()
        .expect("run private GNOME indicator fixture");
    assert!(status.success());
}
