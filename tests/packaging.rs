use std::process::Command;

#[test]
fn installer_target_safety_script_passes() {
    let status = Command::new("bash")
        .arg("packaging/tests/safety.sh")
        .status()
        .expect("run packaging safety checks");
    assert!(status.success());
}
