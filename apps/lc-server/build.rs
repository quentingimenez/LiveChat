//! Build script of `lc-server`: builds the admin console before `rust-embed`
//! embeds it.
//!
//! `apps/lc-server/src/http/admin/static_files.rs` embeds
//! `apps/lc-admin-web/dist/` at compile time, so the Vite output must exist
//! before the crate is compiled. This script runs `npm ci` when needed and
//! `npm run build`, so a plain `cargo test -p lc-server` works on a fresh
//! checkout.
//!
//! A missing Node/npm is only a warning: release builds without Node must keep
//! working (they serve the "console not built" page). Set
//! `LC_ADMIN_WEB_BUILD=0` (or `skip`) when `dist/` is provided by the caller,
//! as the Docker image and the CI test job do.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

/// Environment variable that controls the automatic console build.
const BUILD_ENV: &str = "LC_ADMIN_WEB_BUILD";

fn main() {
    let started = SystemTime::now();
    let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR") else {
        warn("CARGO_MANIFEST_DIR is not set; the admin console was not built");
        return;
    };

    let web = PathBuf::from(manifest_dir).join("..").join("lc-admin-web");
    let web = web.canonicalize().unwrap_or(web);

    // Cargo re-runs this script when the console inputs change, and when the
    // built entry point disappears (a missing watched file always counts as
    // changed, so a deleted `dist/` is rebuilt on the next compilation).
    for watched in [
        "src",
        "index.html",
        "package.json",
        "package-lock.json",
        "vite.config.ts",
        "tsconfig.json",
        "dist/index.html",
    ] {
        println!("cargo:rerun-if-changed={}", web.join(watched).display());
    }
    println!("cargo:rerun-if-env-changed={BUILD_ENV}");

    let mode = env::var(BUILD_ENV).unwrap_or_default();
    if matches!(mode.as_str(), "0" | "skip") {
        return;
    }
    if !matches!(mode.as_str(), "" | "1") {
        warn(&format!(
            "unknown {BUILD_ENV} value `{mode}`; building the admin console anyway"
        ));
    }

    // Build scripts run on the host: on Windows npm is a batch file.
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };

    if needs_install(&web) {
        if let Err(reason) = run_npm(&web, npm, &["ci", "--no-audit", "--no-fund"]) {
            warn_failure(&web, &reason);
            return;
        }
    }

    if let Err(reason) = run_npm(&web, npm, &["run", "build"]) {
        warn_failure(&web, &reason);
        return;
    }
    backdate_entry_point(&web, started);
}

/// Dates the freshly built `dist/index.html` from before this run.
///
/// The file is watched so that a deleted `dist/` is rebuilt, but this script
/// writes it: left as is, it would look newer than the run Cargo recorded and
/// the console would be rebuilt on every compilation. Cargo records the run a
/// few milliseconds before `started`, hence the margin.
fn backdate_entry_point(web: &Path, started: SystemTime) {
    const MARGIN: Duration = Duration::from_secs(2);
    let entry = web.join("dist").join("index.html");
    let stamp = started.checked_sub(MARGIN).unwrap_or(started);
    let result = fs::File::options()
        .write(true)
        .open(&entry)
        .and_then(|file| file.set_modified(stamp));
    if let Err(error) = result {
        warn(&format!(
            "could not reset the date of {}: {error}; the console will be rebuilt on the next              compilation",
            entry.display()
        ));
    }
}

/// `npm ci` is needed when dependencies are absent or older than the lockfile.
fn needs_install(web: &Path) -> bool {
    let Ok(node_modules) = fs::metadata(web.join("node_modules")) else {
        return true;
    };
    let Ok(lockfile) = fs::metadata(web.join("package-lock.json")) else {
        return false;
    };

    match (node_modules.modified(), lockfile.modified()) {
        (Ok(installed), Ok(locked)) => installed < locked,
        _ => false,
    }
}

/// Runs one npm command in the console directory, keeping its output quiet on
/// success. The returned error carries the output tail for the warning.
fn run_npm(web: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(web)
        .output()
        .map_err(|error| format!("`{program}` could not be started: {error}"))?;

    if output.status.success() {
        return Ok(());
    }

    // tsc and Vite are chatty on failure: keep only the last lines.
    let detail = tail(&output.stderr);
    let detail = if detail.is_empty() {
        tail(&output.stdout)
    } else {
        detail
    };
    let detail = if detail.is_empty() {
        output.status.to_string()
    } else {
        detail
    };

    Err(format!("`{program} {}` failed: {detail}", args.join(" ")))
}

/// Last lines of a command output, collapsed into one line for `cargo:warning`.
fn tail(bytes: &[u8]) -> String {
    const MAX_LINES: usize = 8;
    const MAX_CHARS: usize = 600;

    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(MAX_LINES);
    let joined = lines[start..].join(" | ");

    if joined.chars().count() > MAX_CHARS {
        let mut truncated: String = joined.chars().take(MAX_CHARS).collect();
        truncated.push_str("...");
        truncated
    } else {
        joined
    }
}

/// Explains that the embed will be stale or empty, without failing the build.
fn warn_failure(web: &Path, reason: &str) {
    if web.join("dist").join("index.html").is_file() {
        warn(&format!(
            "admin console was NOT rebuilt ({reason}); the embedded console may be stale. \
             Fix Node 24/npm, or run `npm ci && npm run build` in apps/lc-admin-web."
        ));
    } else {
        warn(&format!(
            "admin console not built ({reason}); lc-server will serve the \"console not built\" \
             fallback page. Install Node 24 and npm, or run `npm ci && npm run build` in \
             apps/lc-admin-web. Set {BUILD_ENV}=0 to skip this step."
        ));
    }
}

/// Prints a Cargo warning (never a hard error).
fn warn(message: &str) {
    println!("cargo:warning={message}");
}
