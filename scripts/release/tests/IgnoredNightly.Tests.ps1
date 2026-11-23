# Guard for Q-T2: the `#[ignore]` tests run in the nightly workflow only.
# Protected here: verification-nightly.yml has an `ignored` job gated to
# schedule/workflow_dispatch (never pull_request), it runs the ignored tests of
# lc-media, lc-server and xtask with nextest, and it publishes a per-crate
# report. Removing the job, widening its trigger or dropping a crate fails here.

$workflowPath = Join-Path $script:RepoRoot '.github/workflows/verification-nightly.yml'
$workflow = Get-Content -Raw $workflowPath
$job = [regex]::Match($workflow, '(?ms)^  ignored:[ \t]*\r?\n(.*?)(?=^  [A-Za-z_][A-Za-z0-9_-]*:[ \t]*(?:#.*)?\r?$|\z)').Groups[1].Value
$runStep = [regex]::Match($job, '(?ms)- name: Run the ignored tests[ \t]*\r?\n(.*?)(?=\r?\n      - name:|\z)').Groups[1].Value
$reportStep = [regex]::Match($job, '(?ms)- name: Report the ignored tests per crate[ \t]*\r?\n(.*?)(?=\r?\n      - name:|\z)').Groups[1].Value

Test-Case 'ignored job exists only on the nightly schedule and manual dispatch' {
    Assert-True ($job.Length -gt 0) 'ignored job not found in verification-nightly.yml'

    Assert-True ($job -match "(?m)^    if:.*github\.event_name\s*==\s*'schedule'.*github\.event_name\s*==\s*'workflow_dispatch'") `
        'ignored job must be gated to schedule and workflow_dispatch'
    Assert-True ($job -notmatch 'pull_request') 'ignored job must never be reachable from a pull_request'
    Assert-True ($job -notmatch 'secrets\.') 'ignored job must not use any secret'
}

Test-Case 'ignored job runs the three crates with --run-ignored only' {
    Assert-True ($runStep.Length -gt 0) 'Run the ignored tests step not found'

    Assert-True ($runStep -match 'cargo nextest run\b') 'ignored tests must run under cargo nextest'
    Assert-True ($runStep -match '--run-ignored\s+only') 'ignored tests must use --run-ignored only'
    Assert-True ($runStep -match '--no-fail-fast') 'a failing crate must not stop the other two'
    foreach ($crate in @('lc-media', 'lc-server', 'xtask')) {
        Assert-True ($runStep -match "(?m)\s-p\s+$([regex]::Escape($crate))\b") "run step must select $crate"
    }
}

Test-Case 'ignored job publishes a report with one count per crate' {
    Assert-True ($reportStep.Length -gt 0) 'Report the ignored tests per crate step not found'

    Assert-True ($reportStep -match 'cargo nextest list\b') 'the report must come from nextest, not a hardcoded count'
    Assert-True ($reportStep -match '--run-ignored\s+only') 'the report must list the ignored tests'
    Assert-True ($reportStep -match '\$GITHUB_STEP_SUMMARY') 'the report must show up in the job summary'
    Assert-True ($reportStep -match 'for crate in lc-media lc-server xtask') 'the report must count each of the three crates'
    foreach ($crate in @('lc-media', 'lc-server', 'xtask')) {
        Assert-True ($reportStep -match [regex]::Escape($crate)) "the report must mention $crate"
    }

    Assert-True ($job -match 'name: ignored-tests-report') 'the job must upload the report as an artifact'
    Assert-True ($job -match '(?s)- name: Upload the ignored test report.*if: always\(\)') `
        'the report artifact must upload even when the tests fail'
}
