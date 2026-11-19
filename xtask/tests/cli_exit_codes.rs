#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Process-level exit codes of the `xtask` binary (verification pipeline).
//!
//! The unit tests of `main.rs` cover the parser; these integration tests pin
//! what the process itself does: a CLI error must exit 2 after printing the
//! usage to stderr, and `--help` must exit 0 after printing the usage to
//! stdout. They run the real binary through `CARGO_BIN_EXE_xtask`, the same
//! pattern `kill_recover.rs` uses for `lc-server`.

use std::process::Command;

#[test]
fn unknown_subcommand_exits_2_with_usage_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["definitely-not-a-command-xyzzy"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("xtask :"),
        "stderr must name the tool: {stderr}"
    );
    assert!(
        stderr.contains("USAGE"),
        "stderr must print the usage: {stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "a CLI error must not write to stdout"
    );
}

#[test]
fn help_exits_0_with_usage_on_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("USAGE"),
        "stdout must print the usage: {stdout}"
    );
    assert!(output.stderr.is_empty(), "--help must not write to stderr");
}
