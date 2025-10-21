use assert_cmd::Command;

#[test]
fn cli_displays_help() {
    let mut cmd = Command::cargo_bin("meowdiff").expect("binary exists");
    cmd.arg("--help").assert().success();
}
