# Guard for Q-T2: the `#[ignore]` tests run in the nightly workflow only.
# Protected here: verification-nightly.yml keeps its nightly schedule and manual
# dispatch, the `ignored` job stays unreachable from a pull_request, runs the
# ignored tests of lc-media, lc-server and xtask under nextest without muting a
# failure, and publishes the per-crate report. Removing the schedule, widening
# the job, dropping a crate or the jq `ignored` filter, neutralising the run
# step or changing the artifact's always() all fail here.

$workflowPath = Join-Path $script:RepoRoot '.github/workflows/verification-nightly.yml'
$workflow = Get-Content -Raw $workflowPath
$onBlock = [regex]::Match($workflow, '(?ms)^on:[ \t]*\r?\n(.*?)(?=^[A-Za-z_][^\r\n:]*:[ \t]*(?:#.*)?\r?$|\z)').Groups[1].Value
$job = [regex]::Match($workflow, '(?ms)^  ignored:[ \t]*\r?\n(.*?)(?=^  [A-Za-z_][A-Za-z0-9_-]*:[ \t]*(?:#.*)?\r?$|\z)').Groups[1].Value
$runStep = [regex]::Match($job, '(?ms)^      - name: Run the ignored tests[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
$reportStep = [regex]::Match($job, '(?ms)^      - name: Report the ignored tests per crate[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
$uploadStep = [regex]::Match($job, '(?ms)^      - name: Upload the ignored test report[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value

Test-Case 'ignored job keeps the nightly schedule and manual dispatch trigger' {
    Assert-True ($onBlock.Length -gt 0) 'the workflow on: block not found'
    Assert-True ($onBlock -match '(?m)^  schedule:[ \t]*\r?$') 'verification-nightly.yml must keep the schedule trigger'
    Assert-True ($onBlock -match "(?m)^\s*-\s*cron:\s*'30 3 \* \* \*'[ \t]*\r?$") 'the nightly cron must stay at 30 3 * * *'
    Assert-True ($onBlock -match '(?m)^  workflow_dispatch:[ \t]*\r?$') 'verification-nightly.yml must keep workflow_dispatch'
}

Test-Case 'ignored job is named, bounded, and gated to nightly/manual runs' {
    Assert-True ($job.Length -gt 0) 'ignored job not found in verification-nightly.yml'
    Assert-True ($job -match '(?m)^    name: ignored tests[ \t]*\r?$') 'the job must stay named "ignored tests"'
    Assert-True ($job -match '(?m)^    timeout-minutes: 60[ \t]*\r?$') 'the job must keep its 60 minute timeout'
    Assert-True ($job -match '(?m)^      LC_ADMIN_WEB_BUILD:\s*"0"[ \t]*\r?$') 'the job must keep LC_ADMIN_WEB_BUILD=0'

    $ifLine = [regex]::Match($job, '(?m)^    if:.*$').Value
    Assert-True ($ifLine -match 'schedule') 'the job if: must allow the schedule event'
    Assert-True ($ifLine -match 'workflow_dispatch') 'the job if: must allow workflow_dispatch'
    Assert-Equal 2 ([regex]::Matches($ifLine, 'github\.event_name').Count) `
        'the job if: must test exactly the schedule and workflow_dispatch events'
    Assert-True ($ifLine -notmatch 'pull_request') 'the job if: must not mention pull_request'
    Assert-True ($job -notmatch "github\.event_name\s*==\s*'pull_request'") 'the job must never be reachable from a pull_request'
    Assert-True ($job -notmatch '(?m)^\s*pull_request\s*:') 'the job must not declare a pull_request trigger'
    Assert-True ($job -notmatch 'secrets\.') 'the job must not use any secret'
    Assert-True ($job -notmatch 'continue-on-error') 'the job must not neutralise a failing step'
}

Test-Case 'ignored job runs the three crates without muting nextest' {
    Assert-True ($runStep.Length -gt 0) 'Run the ignored tests step not found'
    Assert-True ($runStep -match 'cargo nextest run\b') 'ignored tests must run under cargo nextest'
    Assert-True ($runStep -match '--run-ignored\s+only') 'ignored tests must use --run-ignored only'
    Assert-True ($runStep -match '--no-fail-fast') 'a failing crate must not stop the other two'
    foreach ($crate in @('lc-media', 'lc-server', 'xtask')) {
        Assert-True ($runStep -match "(?m)\s-p\s+$([regex]::Escape($crate))\b") "run step must select $crate"
    }
    Assert-True ($runStep -notmatch 'continue-on-error') 'the run step must not continue on error'
    Assert-True ($runStep -notmatch '(?m)^\s+if\s*:') 'the run step must stay unconditionally gated on success'
    Assert-True ($runStep -notmatch '\|\|\s*true\b') 'the run step must not swallow failures with || true'
    Assert-True ($runStep -notmatch '(?m)^\s*set\s+\+e\b') 'the run step must not disable set -e'
}

Test-Case 'ignored job publishes a per-crate report with the real inventory' {
    Assert-True ($reportStep.Length -gt 0) 'Report the ignored tests per crate step not found'
    Assert-True ($reportStep -match 'cargo nextest list\b') 'the report must come from nextest, not a hardcoded count'
    Assert-True ($reportStep -match '--run-ignored\s+only') 'the report must list the ignored tests'
    Assert-True ($reportStep -match '(?m)^[ \t]*\|[ \t]*select\(\s*\.value\.ignored\s*\)[ \t]*\r?$') 'the jq filter must keep only ignored testcases'
    Assert-True ($reportStep -match '\$GITHUB_STEP_SUMMARY') 'the report must show up in the job summary'
    Assert-True ($reportStep -match '(?m)^[ \t]*for crate in lc-media lc-server xtask; do[ \t]*\r?$') 'the report must count each of the three crates'
    foreach ($crate in @('lc-media', 'lc-server', 'xtask')) {
        Assert-True ($reportStep -match "(?m)\s-p\s+$([regex]::Escape($crate))\b") "the report list must select $crate"
    }

    $counts = @{ 'lc-media' = 0; 'lc-server' = 0; 'xtask' = 0 }
    $ignoredLines = @(& git -C $script:RepoRoot grep -n '^\s*#\[ignore' -- '*.rs' 2>$null)
    Assert-True ($ignoredLines.Count -gt 0) 'git grep found no #[ignore] attributes'
    foreach ($line in $ignoredLines) {
        $path = ($line -split ':', 2)[0]
        if ($path -like 'crates/lc-media/*') { $counts['lc-media'] = $counts['lc-media'] + 1 }
        elseif ($path -like 'apps/lc-server/*') { $counts['lc-server'] = $counts['lc-server'] + 1 }
        elseif ($path -like 'xtask/*') { $counts['xtask'] = $counts['xtask'] + 1 }
    }
    Assert-Equal 39 $counts['lc-media'] 'lc-media ignored-test inventory changed; update the documented counts'
    Assert-Equal 11 $counts['lc-server'] 'lc-server ignored-test inventory changed; update the documented counts'
    Assert-Equal 2 $counts['xtask'] 'xtask ignored-test inventory changed; update the documented counts'
    Assert-Equal 52 (($counts.Values | Measure-Object -Sum).Sum) 'the ignored-test total must stay 52'
}

Test-Case 'ignored job uploads the report even when a test fails' {
    Assert-True ($uploadStep.Length -gt 0) 'Upload the ignored test report step not found'
    Assert-True ($uploadStep -match '(?m)^\s+name:\s*ignored-tests-report[ \t]*\r?$') 'the job must upload the report as an artifact'
    Assert-True ($uploadStep -match '(?m)^\s+if:\s*always\(\)[ \t]*\r?$') 'the report artifact must upload even when the tests fail'
    foreach ($artifactPath in @('target/ignored-tests.json', 'target/ignored-tests.md', 'target/ignored-tests.txt')) {
        Assert-True ($uploadStep -match [regex]::Escape($artifactPath)) "the artifact must include $artifactPath"
    }
}
