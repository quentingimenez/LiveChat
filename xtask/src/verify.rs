//! Local verification pipeline for the LiveChat workspace.
//!
//! `verify` replays the same battery as CI (`rust.yml`), stage by stage, and
//! prints exactly one verdict line per stage — `PASS <name>` or `FAIL <name>`
//! (with an optional parenthesised note) — followed by a single summary line.
//! The process exit code is the number of failed stages. Stages run
//! sequentially and each tool's own output is streamed live, so the command
//! must be invoked from the workspace root.
//!
//! Stages, in order:
//!
//! 1. `fmt`: `cargo fmt --all -- --check`
//! 2. `clippy`: `cargo clippy --workspace --all-targets --all-features
//!    --locked -- -D warnings`
//! 3. `tests`: `cargo nextest run --workspace --all-features --locked
//!    --no-fail-fast` — no fallback: a missing cargo-nextest fails the stage
//! 4. `doctests`: `cargo test --workspace --doc --locked` (nextest does not
//!    run doctests)
//! 5. `deny`: `cargo deny check`
//!
//! `--offline` inserts `--offline` into every cargo invocation that accepts
//! it (clippy, tests, doctests, deny — not fmt). The exact cargo arguments of
//! the five CI stages live in [`stage_argv`], copied verbatim from
//! `rust.yml`, so a unit test can pin them to the workflow.
//!
//! `--mutants` appends a sixth stage: `cargo mutants --in-place --in-diff`
//! over the Rust changes since `origin/main`. The run happens in place on
//! purpose: the copy mode skips the gitignored `apps/lc-admin-web/dist` and
//! `node_modules`, so the `lc-server` build script would call npm. Never
//! interrupt a mutants run: it mutates the working tree and restores it
//! itself; an interrupted run leaves a mutated file behind.

use std::fs;
use std::process::{Command, Stdio};

/// Options of the `verify` subcommand.
#[derive(Debug)]
pub struct Options {
    /// `--offline`: every cargo stage that accepts it runs offline.
    pub offline: bool,
    /// `--mutants`: appends the `mutants` stage.
    pub mutants: bool,
}

/// Verdict of a single stage, rendered as exactly one line.
#[derive(Debug)]
enum Verdict {
    /// Stage succeeded, with an optional note on the stage line.
    Pass(Option<String>),
    /// Stage failed, with an optional note on the stage line.
    Fail(Option<String>),
}

impl Verdict {
    /// Label printed before the stage name.
    fn label(&self) -> &'static str {
        match self {
            Verdict::Pass(_) => "PASS",
            Verdict::Fail(_) => "FAIL",
        }
    }
}

/// One pipeline stage: display name and runner. The runner receives the
/// `--offline` flag; the `mutants` stage ignores it.
struct Stage {
    /// Name used in the verdict line.
    name: &'static str,
    /// Runs the stage and returns its verdict.
    run: fn(offline: bool) -> Verdict,
}

/// Runs the verification pipeline and returns the process exit code:
/// the number of failed stages (`0` when every stage passes).
pub fn execute(options: Options) -> u8 {
    let mut stages = vec![
        Stage {
            name: "fmt",
            run: stage_fmt,
        },
        Stage {
            name: "clippy",
            run: stage_clippy,
        },
        Stage {
            name: "tests",
            run: stage_tests,
        },
        Stage {
            name: "doctests",
            run: stage_doctests,
        },
        Stage {
            name: "deny",
            run: stage_deny,
        },
    ];
    if options.mutants {
        stages.push(Stage {
            name: "mutants",
            run: stage_mutants,
        });
    }

    let mut passed: Vec<&'static str> = Vec::new();
    let mut failed: Vec<&'static str> = Vec::new();

    for stage in &stages {
        let verdict = (stage.run)(options.offline);
        print_verdict(stage.name, &verdict);
        match verdict {
            Verdict::Pass(_) => passed.push(stage.name),
            Verdict::Fail(_) => failed.push(stage.name),
        }
    }

    if failed.is_empty() {
        println!("verify: {} passed, 0 failed", passed.len());
    } else {
        println!(
            "verify: {} passed, {} failed ({})",
            passed.len(),
            failed.len(),
            failed.join(", ")
        );
    }

    u8::try_from(failed.len()).unwrap_or(u8::MAX)
}

/// Cargo arguments of a stage, copied verbatim from `rust.yml` so the local
/// battery stays pinned to CI. The pin is enforced by the
/// `stage_argv_mirrors_rust_yml` test, which reads `rust.yml` and fails if
/// any stage command line drifts away from it. `--offline` is inserted only
/// when `offline` is true and the stage accepts it (not `fmt`); an unknown
/// stage name yields an empty argument list.
pub(crate) fn stage_argv(stage: &str, offline: bool) -> Vec<&'static str> {
    let mut argv: Vec<&'static str> = match stage {
        "fmt" => return vec!["fmt", "--all", "--", "--check"],
        "clippy" => vec![
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
        ],
        "tests" => vec![
            "nextest",
            "run",
            "--workspace",
            "--all-features",
            "--locked",
        ],
        "doctests" => vec!["test", "--workspace", "--doc", "--locked"],
        "deny" => vec!["deny"],
        _ => return Vec::new(),
    };
    if offline {
        argv.push("--offline");
    }
    match stage {
        "clippy" => argv.extend(["--", "-D", "warnings"]),
        "tests" => argv.push("--no-fail-fast"),
        "deny" => argv.push("check"),
        _ => {}
    }
    argv
}

/// `fmt` stage: `cargo fmt --all -- --check`.
fn stage_fmt(offline: bool) -> Verdict {
    verdict_of(run_cargo(&stage_argv("fmt", offline)))
}

/// `clippy` stage: `cargo clippy --workspace --all-targets --all-features
/// --locked -- -D warnings`.
fn stage_clippy(offline: bool) -> Verdict {
    verdict_of(run_cargo(&stage_argv("clippy", offline)))
}

/// `tests` stage: `cargo nextest run --workspace --all-features --locked
/// --no-fail-fast`. No fallback: a missing cargo-nextest fails the stage
/// rather than silently running a weaker battery than CI.
fn stage_tests(offline: bool) -> Verdict {
    if !cargo_subcommand_installed("nextest") {
        return Verdict::Fail(Some("cargo-nextest not installed".to_string()));
    }
    verdict_of(run_cargo(&stage_argv("tests", offline)))
}

/// `doctests` stage: `cargo test --workspace --doc --locked` — nextest does
/// not run doctests.
fn stage_doctests(offline: bool) -> Verdict {
    verdict_of(run_cargo(&stage_argv("doctests", offline)))
}

/// `deny` stage: `cargo deny check`. No skip: CI (`rust.yml`) enforces it,
/// so a missing cargo-deny fails the stage.
fn stage_deny(offline: bool) -> Verdict {
    if !cargo_subcommand_installed("deny") {
        return Verdict::Fail(Some("cargo-deny not installed".to_string()));
    }
    verdict_of(run_cargo(&stage_argv("deny", offline)))
}

/// `mutants` stage (only with `--mutants`): `cargo mutants` over the Rust
/// changes since `origin/main`.
///
/// The diff feeds `--in-diff`, so only the changed code is mutated; the run
/// is in place because the copy mode skips the gitignored
/// `apps/lc-admin-web/dist` and `node_modules`, which would make the
/// `lc-server` build script call npm. This stage can take a long time;
/// never interrupt it — cargo-mutants restores the mutated files itself, an
/// interrupted run does not.
fn stage_mutants(_offline: bool) -> Verdict {
    if !cargo_subcommand_installed("mutants") {
        return Verdict::Fail(Some("cargo-mutants not installed".to_string()));
    }

    // The merge base of HEAD and origin/main scopes the diff.
    let base = match git_output(&["merge-base", "HEAD", "origin/main"]) {
        Ok(base) => base.trim().to_string(),
        Err(reason) => return Verdict::Fail(Some(reason)),
    };

    // The Rust diff since that base, written for `--in-diff`. The pathspec
    // is one argument: no shell ever sees the `*`.
    let diff = match git_output(&["diff", base.as_str(), "--", "*.rs"]) {
        Ok(diff) => diff,
        Err(reason) => return Verdict::Fail(Some(reason)),
    };
    if let Err(error) = fs::create_dir_all("target/verify") {
        return Verdict::Fail(Some(format!("could not create target/verify: {error}")));
    }
    if let Err(error) = fs::write("target/verify/mutants.diff", diff.as_bytes()) {
        return Verdict::Fail(Some(format!(
            "could not write target/verify/mutants.diff: {error}"
        )));
    }

    // Nothing changed in Rust since origin/main: nothing to mutate.
    if diff.is_empty() {
        return Verdict::Pass(Some("no Rust change since origin/main".to_string()));
    }

    let status = Command::new("cargo")
        .args([
            "mutants",
            "--in-place",
            "--in-diff",
            "target/verify/mutants.diff",
            "--test-tool",
            "nextest",
            "--output",
            "target/verify",
        ])
        .status();
    let success = match status {
        Ok(status) => status.success(),
        Err(error) => {
            eprintln!("verify: could not run `cargo mutants`: {error}");
            false
        }
    };
    if success {
        Verdict::Pass(None)
    } else {
        Verdict::Fail(Some("see target/verify/mutants.out/missed.txt".to_string()))
    }
}

/// Maps a stage success flag to a bare `Pass`/`Fail` verdict.
fn verdict_of(success: bool) -> Verdict {
    if success {
        Verdict::Pass(None)
    } else {
        Verdict::Fail(None)
    }
}

/// Prints the single verdict line of a stage.
fn print_verdict(name: &str, verdict: &Verdict) {
    let note = match verdict {
        Verdict::Pass(note) => note.as_deref(),
        Verdict::Fail(note) => note.as_deref(),
    };
    match note {
        Some(note) => println!("{} {} ({})", verdict.label(), name, note),
        None => println!("{} {}", verdict.label(), name),
    }
}

/// Runs `cargo <args>…` in the current directory, streaming the tool's
/// output live. Returns `true` when the command exits with status `0`; a
/// stage whose command cannot even be spawned counts as failed.
fn run_cargo(args: &[&str]) -> bool {
    match Command::new("cargo").args(args).status() {
        Ok(status) => status.success(),
        Err(error) => {
            eprintln!("verify: could not run `cargo {}`: {error}", args.join(" "));
            false
        }
    }
}

/// Runs `git <args>…` and returns its stdout, or a one-line reason fit for
/// a verdict note when the command fails.
fn git_output(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|error| format!("could not run git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.lines().map(str::trim).find(|line| !line.is_empty());
        return match detail {
            Some(detail) => Err(format!("git {} failed: {detail}", args.join(" "))),
            None => Err(format!("git {} failed", args.join(" "))),
        };
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Probes `cargo <subcommand> --version` quietly to detect an installed
/// cargo subcommand.
fn cargo_subcommand_installed(subcommand: &str) -> bool {
    Command::new("cargo")
        .arg(subcommand)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::stage_argv;

    #[test]
    fn stage_argv_mirrors_rust_yml() {
        assert_eq!(stage_argv("fmt", false), ["fmt", "--all", "--", "--check"]);
        assert_eq!(stage_argv("fmt", true), ["fmt", "--all", "--", "--check"]);
        assert_eq!(
            stage_argv("clippy", false),
            [
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--locked",
                "--",
                "-D",
                "warnings",
            ]
        );
        assert_eq!(
            stage_argv("clippy", true),
            [
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--locked",
                "--offline",
                "--",
                "-D",
                "warnings",
            ]
        );
        assert_eq!(
            stage_argv("tests", false),
            [
                "nextest",
                "run",
                "--workspace",
                "--all-features",
                "--locked",
                "--no-fail-fast",
            ]
        );
        assert_eq!(
            stage_argv("tests", true),
            [
                "nextest",
                "run",
                "--workspace",
                "--all-features",
                "--locked",
                "--offline",
                "--no-fail-fast",
            ]
        );
        assert_eq!(
            stage_argv("doctests", false),
            ["test", "--workspace", "--doc", "--locked"]
        );
        assert_eq!(
            stage_argv("doctests", true),
            ["test", "--workspace", "--doc", "--locked", "--offline"]
        );
        assert_eq!(stage_argv("deny", false), ["deny", "check"]);
        assert_eq!(stage_argv("deny", true), ["deny", "--offline", "check"]);

        // The hardcoded arrays above pin the shape of `stage_argv`; this
        // second half pins it to CI itself: each non-offline command line
        // must appear verbatim in `.github/workflows/rust.yml`, so a future
        // edit of one side without the other fails here instead of letting
        // the local battery drift away from CI. `--offline` variants are
        // local-only (CI never needs them), so they are not searched.
        const RUST_YML: &str = include_str!("../../.github/workflows/rust.yml");

        fn assert_pinned_to_rust_yml(stage: &str, argv: &[&str]) {
            let command = format!("cargo {}", argv.join(" "));
            assert!(
                RUST_YML.contains(&command),
                "stage {stage} runs `{command}`, which is not pinned verbatim in .github/workflows/rust.yml"
            );
        }

        assert_pinned_to_rust_yml("fmt", &stage_argv("fmt", false));
        assert_pinned_to_rust_yml("clippy", &stage_argv("clippy", false));
        assert_pinned_to_rust_yml("tests", &stage_argv("tests", false));
        assert_pinned_to_rust_yml("doctests", &stage_argv("doctests", false));
        // `deny` has no literal cargo line in rust.yml (CI runs the pinned
        // EmbarkStudios cargo-deny-action), so pin its step name instead:
        // the action is documented as running `cargo deny check`.
        assert!(RUST_YML.contains("cargo deny check"));
    }
}
