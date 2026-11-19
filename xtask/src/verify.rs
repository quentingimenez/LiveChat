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
use std::io::Write;
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
pub(crate) struct Stage {
    /// Name used in the verdict line.
    name: &'static str,
    /// Runs the stage and returns its verdict.
    run: fn(offline: bool) -> Verdict,
}

/// The five CI stages, plus the `mutants` stage when `--mutants` was
/// requested.
pub(crate) fn default_stages(mutants: bool) -> Vec<Stage> {
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
    if mutants {
        stages.push(Stage {
            name: "mutants",
            run: stage_mutants,
        });
    }
    stages
}

/// Runs every stage sequentially, writing one verdict line per stage plus
/// the summary line to `out`, and returns the number of failed stages as
/// the exit code. `main` is the only caller: it also owns the `stdout` sink.
pub(crate) fn drive(stages: &[Stage], offline: bool, out: &mut impl Write) -> u8 {
    let mut passed: Vec<&'static str> = Vec::new();
    let mut failed: Vec<&'static str> = Vec::new();

    for stage in stages {
        let verdict = (stage.run)(offline);
        writeln!(out, "{}", verdict_line(stage.name, &verdict)).ok();
        match verdict {
            Verdict::Pass(_) => passed.push(stage.name),
            Verdict::Fail(_) => failed.push(stage.name),
        }
    }

    if failed.is_empty() {
        writeln!(out, "verify: {} passed, 0 failed", passed.len()).ok();
    } else {
        writeln!(
            out,
            "verify: {} passed, {} failed ({})",
            passed.len(),
            failed.len(),
            failed.join(", ")
        )
        .ok();
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

/// Gate shared by the tool-dependent stages: a missing tool is a failed
/// stage, with a note naming the tool.
fn require_tool(tool: &str, installed: bool) -> Option<Verdict> {
    if installed {
        None
    } else {
        Some(Verdict::Fail(Some(format!("{tool} not installed"))))
    }
}

/// `tests` stage: `cargo nextest run --workspace --all-features --locked
/// --no-fail-fast`. No fallback: a missing cargo-nextest fails the stage
/// rather than silently running a weaker battery than CI.
fn stage_tests(offline: bool) -> Verdict {
    if let Some(fail) = require_tool(
        "cargo-nextest",
        probe_installed("cargo", &["nextest", "--version"]),
    ) {
        return fail;
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
    if let Some(fail) = require_tool(
        "cargo-deny",
        probe_installed("cargo", &["deny", "--version"]),
    ) {
        return fail;
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
    if let Some(fail) = require_tool(
        "cargo-mutants",
        probe_installed("cargo", &["mutants", "--version"]),
    ) {
        return fail;
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

/// Renders the single verdict line of a stage: `PASS <name>` or
/// `FAIL <name>`, with the optional note in parentheses.
fn verdict_line(name: &str, verdict: &Verdict) -> String {
    let note = match verdict {
        Verdict::Pass(note) => note.as_deref(),
        Verdict::Fail(note) => note.as_deref(),
    };
    match note {
        Some(note) => format!("{} {} ({})", verdict.label(), name, note),
        None => format!("{} {}", verdict.label(), name),
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

/// Probes `program args…` quietly: true when the program runs and exits 0.
/// Used for `cargo <subcommand> --version` door checks; kept as a pure
/// program probe so the decision logic is testable without depending on
/// which tools happen to be installed in the environment.
fn probe_installed(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{
        default_stages, drive, git_output, probe_installed, require_tool, run_cargo, stage_argv,
        verdict_line, Stage, Verdict,
    };

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

    fn always_pass(_offline: bool) -> Verdict {
        Verdict::Pass(None)
    }

    fn always_fail(_offline: bool) -> Verdict {
        Verdict::Fail(None)
    }

    fn panic_unless_offline(offline: bool) -> Verdict {
        assert!(offline, "the runner must receive the offline flag verbatim");
        Verdict::Pass(None)
    }

    fn mutants_missed(_offline: bool) -> Verdict {
        Verdict::Fail(Some("see target/verify/mutants.out/missed.txt".to_owned()))
    }

    fn run_drive(stages: &[Stage], offline: bool) -> (String, u8) {
        let mut out: Vec<u8> = Vec::new();
        let code = drive(stages, offline, &mut out);
        (String::from_utf8(out).unwrap(), code)
    }

    #[test]
    fn default_stages_without_mutants_are_the_five_ci_stages() {
        let names: Vec<&'static str> = default_stages(false).iter().map(|s| s.name).collect();
        assert_eq!(names, ["fmt", "clippy", "tests", "doctests", "deny"]);
    }

    #[test]
    fn default_stages_with_mutants_appends_the_mutants_stage() {
        let names: Vec<&'static str> = default_stages(true).iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            ["fmt", "clippy", "tests", "doctests", "deny", "mutants"]
        );
    }

    #[test]
    fn drive_prints_verdict_lines_and_the_plural_failure_summary() {
        let stages = vec![
            Stage {
                name: "fmt",
                run: always_pass,
            },
            Stage {
                name: "tests",
                run: always_fail,
            },
            Stage {
                name: "deny",
                run: always_fail,
            },
        ];
        let (output, code) = run_drive(&stages, true);
        assert_eq!(
            output,
            "PASS fmt\nFAIL tests\nFAIL deny\nverify: 1 passed, 2 failed (tests, deny)\n"
        );
        assert_eq!(code, 2);
    }

    #[test]
    fn drive_prints_the_zero_failure_summary_when_every_stage_passes() {
        let stages = vec![
            Stage {
                name: "fmt",
                run: always_pass,
            },
            Stage {
                name: "deny",
                run: always_pass,
            },
        ];
        let (output, code) = run_drive(&stages, false);
        assert_eq!(output, "PASS fmt\nPASS deny\nverify: 2 passed, 0 failed\n");
        assert_eq!(code, 0);
    }

    #[test]
    fn drive_renders_verdict_notes_verbatim() {
        let stages = vec![
            Stage {
                name: "fmt",
                run: always_pass,
            },
            Stage {
                name: "tests",
                run: always_fail,
            },
            Stage {
                name: "mutants",
                run: mutants_missed,
            },
        ];
        let (output, code) = run_drive(&stages, true);
        assert_eq!(
            output,
            "PASS fmt\nFAIL tests\n\
             FAIL mutants (see target/verify/mutants.out/missed.txt)\n\
             verify: 1 passed, 2 failed (tests, mutants)\n"
        );
        assert_eq!(code, 2);
    }

    #[test]
    fn drive_forwards_the_offline_flag_to_every_runner() {
        let stages = vec![
            Stage {
                name: "probe-a",
                run: panic_unless_offline,
            },
            Stage {
                name: "probe-b",
                run: panic_unless_offline,
            },
        ];
        let (output, code) = run_drive(&stages, true);
        assert_eq!(
            output,
            "PASS probe-a\nPASS probe-b\nverify: 2 passed, 0 failed\n"
        );
        assert_eq!(code, 0);
    }

    #[test]
    fn require_tool_lets_an_installed_tool_through() {
        assert!(require_tool("cargo-nextest", true).is_none());
    }

    #[test]
    fn require_tool_fails_a_missing_tool_with_its_name() {
        match require_tool("cargo-nextest", false) {
            Some(Verdict::Fail(Some(note))) => assert_eq!(note, "cargo-nextest not installed"),
            other => panic!("expected a failing verdict with a note, got {other:?}"),
        }
    }

    #[test]
    fn run_cargo_reports_a_missing_subcommand_as_a_failure() {
        assert!(!run_cargo(&["definitely-not-a-subcommand-xyzzy"]));
    }

    #[test]
    fn run_cargo_reports_a_builtin_subcommand_as_a_success() {
        // Measured locally: `cargo version` prints the version and exits 0
        // (no builtin cargo subcommand answers `--version`).
        assert!(run_cargo(&["version"]));
    }

    #[test]
    fn probe_installed_detects_a_program_that_answers_zero() {
        // The test binary itself is a libtest harness: `--list` always
        // exits 0, cross-platform, under both `cargo test` and nextest.
        let exe = std::env::current_exe().unwrap();
        let exe = exe.to_str().unwrap();
        assert!(probe_installed(exe, &["--list"]));
    }

    #[test]
    fn probe_installed_rejects_a_program_that_answers_nonzero() {
        // The same libtest harness exits nonzero on an unknown flag.
        let exe = std::env::current_exe().unwrap();
        let exe = exe.to_str().unwrap();
        assert!(!probe_installed(exe, &["--definitely-not-a-libtest-flag"]));
    }

    #[test]
    fn probe_installed_rejects_a_missing_program() {
        // Spawning a nonexistent program is an Err, not a nonzero exit.
        assert!(!probe_installed("definitely-not-a-program-xyz", &[]));
    }

    #[test]
    fn probe_installed_rejects_an_unknown_cargo_subcommand() {
        // Door-level semantics: cargo exits 101 on an unknown subcommand,
        // deterministically.
        assert!(!probe_installed(
            "cargo",
            &["definitely-not-a-subcommand-xyzzy", "--version"]
        ));
    }

    #[test]
    fn git_output_returns_stdout_on_success() {
        // Hermetic fixture: a throwaway repository in a tempdir, reached
        // through `git -C`, so the test passes both in the work tree and in
        // cargo-mutants' source copy (which carries no .git).
        let repo = tempfile::tempdir().unwrap();
        let path = repo.path().to_str().unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "--quiet", path])
            .status()
            .unwrap();
        assert!(init.success(), "git init must succeed for the fixture");
        let inside = git_output(&["-C", path, "rev-parse", "--is-inside-work-tree"]).unwrap();
        assert_eq!(inside.trim(), "true");
    }

    #[test]
    fn git_output_surfaces_the_first_stderr_line_on_failure() {
        let error = git_output(&["definitely-not-a-git-command-xyzzy"])
            .expect_err("an unknown subcommand must fail");
        assert!(
            error.starts_with("git definitely-not-a-git-command-xyzzy failed: "),
            "the error must carry the git prefix and the failed subcommand: {error}"
        );
        assert!(
            error.contains("is not a git command"),
            "the first non-empty stderr line must surface: {error}"
        );
    }

    #[test]
    fn git_output_fails_on_an_unknown_revision() {
        let result = git_output(&[
            "rev-parse",
            "--verify",
            "refs/heads/definitely-not-a-branch-xyzzy",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn verdict_line_renders_every_verdict_shape() {
        assert_eq!(verdict_line("fmt", &Verdict::Pass(None)), "PASS fmt");
        assert_eq!(
            verdict_line("fmt", &Verdict::Pass(Some("n".to_owned()))),
            "PASS fmt (n)"
        );
        assert_eq!(verdict_line("fmt", &Verdict::Fail(None)), "FAIL fmt");
        assert_eq!(
            verdict_line("fmt", &Verdict::Fail(Some("why".to_owned()))),
            "FAIL fmt (why)"
        );
    }
}
