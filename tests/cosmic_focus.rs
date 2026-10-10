// SPDX-License-Identifier: MIT

#[test]
fn cosmic_focus_protocol_regressions() {
    let status = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/scripts/cosmic_focus_test.py"
        ))
        .arg(env!("CARGO_BIN_EXE_computer-use-linux-cosmic"))
        .status()
        .expect("run COSMIC wire protocol fixture");
    assert!(status.success());
}
