# Guard for Q-T2: the `#[ignore]` tests run in the nightly workflow only.
# Protected here: verification-nightly.yml keeps its nightly schedule and manual
# dispatch, the `ignored` job stays unreachable from a pull_request, runs the
# ignored tests of lc-media, lc-server and xtask under nextest without muting a
# failure, and publishes the per-crate report. Removing the schedule, widening
# the job, dropping a crate, commenting out the run/artifact, neutralising the
# run step, or adding an ignored test outside these three crates all fail here.
#
# Any new `#[ignore]` suite must be added to the job's `-p` list and to the
# counts below; the nightly report is the only place those tests run.

$workflowPath = Join-Path $script:RepoRoot '.github/workflows/verification-nightly.yml'
$workflow = Get-Content -Raw $workflowPath
$job = [regex]::Match($workflow, '(?ms)^  ignored:[ \t]*\r?\n(.*?)(?=^  [A-Za-z_][A-Za-z0-9_-]*:[ \t]*(?:#.*)?\r?$|\z)').Groups[1].Value
$runStep = [regex]::Match($job, '(?ms)^      - name: Run the ignored tests[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
$reportStep = [regex]::Match($job, '(?ms)^      - name: Report the ignored tests per crate[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
$uploadStep = [regex]::Match($job, '(?ms)^      - name: Upload the ignored test report[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value

# YAML comments and shell comments inside `run:` scalars are inert: drop them
# before matching so a commented-out command or path cannot satisfy an assertion.
function Remove-CommentLines {
    param([string]$Text)
    $kept = foreach ($line in ($Text -split "\r?\n")) {
        if ($line -notmatch '^\s*#') { $line }
    }
    return ($kept -join "`n")
}

$onBlock = [regex]::Match($workflow, '(?ms)^on:[ \t]*\r?\n(.*?)(?=^[A-Za-z_][^\r\n:]*:[ \t]*(?:#.*)?\r?$|\z)').Groups[1].Value
$onNoComments = Remove-CommentLines $onBlock
$jobNoComments = Remove-CommentLines $job
$runNoComments = Remove-CommentLines $runStep
$reportNoComments = Remove-CommentLines $reportStep
$uploadNoComments = Remove-CommentLines $uploadStep
$ifKeyPattern = '(?m)^[ \t]*["'']?if["'']?\s*:'

Test-Case 'ignored job keeps the nightly schedule and manual dispatch trigger' {
    Assert-True ($onNoComments.Length -gt 0) 'the workflow on: block not found'
    Assert-True ($onNoComments -match '(?m)^  schedule:[ \t]*\r?$') 'verification-nightly.yml must keep the schedule trigger'
    Assert-True ($onNoComments -match "(?m)^\s*-\s*cron:\s*'30 3 \* \* \*'[ \t]*\r?$") 'the nightly cron must stay at 30 3 * * *'
    Assert-True ($onNoComments -match '(?m)^  workflow_dispatch:[ \t]*\r?$') 'verification-nightly.yml must keep workflow_dispatch'
}

Test-Case 'ignored job is named, bounded, and gated to nightly/manual runs' {
    Assert-True ($job.Length -gt 0) 'ignored job not found in verification-nightly.yml'
    Assert-True ($jobNoComments -match '(?m)^    name: ignored tests[ \t]*\r?$') 'the job must stay named "ignored tests"'
    Assert-True ($jobNoComments -match '(?m)^    timeout-minutes: 60[ \t]*\r?$') 'the job must keep its 60 minute timeout'
    Assert-True ($jobNoComments -match '(?m)^      LC_ADMIN_WEB_BUILD:\s*"0"[ \t]*\r?$') 'the job must keep LC_ADMIN_WEB_BUILD=0'

    $ifLine = [regex]::Match($jobNoComments, '(?m)^    ["'']?if["'']?\s*:[^\r\n]*').Value
    Assert-True ($ifLine.Length -gt 0) 'the job must keep its if: gate'
    Assert-True ($ifLine -match 'schedule') 'the job if: must allow the schedule event'
    Assert-True ($ifLine -match 'workflow_dispatch') 'the job if: must allow workflow_dispatch'
    Assert-Equal 2 ([regex]::Matches($ifLine, 'github\.event_name').Count) `
        'the job if: must test exactly the schedule and workflow_dispatch events'
    Assert-True ($ifLine -notmatch 'pull_request') 'the job if: must not mention pull_request'
    Assert-True ($jobNoComments -notmatch "github\.event_name\s*==\s*'pull_request'") 'the job must never be reachable from a pull_request'
    Assert-True ($jobNoComments -notmatch '(?m)^\s*pull_request\s*:') 'the job must not declare a pull_request trigger'
    Assert-True ($jobNoComments -notmatch 'secrets\.') 'the job must not use any secret'
    Assert-True ($jobNoComments -notmatch 'continue-on-error') 'the job must not neutralise a failing step'
}

Test-Case 'ignored job runs the three crates without muting nextest' {
    Assert-True ($runStep.Length -gt 0) 'Run the ignored tests step not found'
    Assert-Equal 1 ([regex]::Matches($jobNoComments, 'cargo nextest run\b').Count) `
        'exactly one non-commented cargo nextest run is allowed in the job'
    Assert-True ($runNoComments -match 'cargo nextest run\b') 'ignored tests must run under cargo nextest'
    Assert-True ($runNoComments -match '--run-ignored\s+only') 'ignored tests must use --run-ignored only'
    Assert-True ($runNoComments -match '--no-fail-fast') 'a failing crate must not stop the other two'
    foreach ($crate in @('lc-media', 'lc-server', 'xtask')) {
        Assert-True ($runNoComments -match "(?m)\s-p\s+$([regex]::Escape($crate))\b") "run step must select $crate"
    }
    Assert-True ($runNoComments -notmatch 'continue-on-error') 'the run step must not continue on error'
    Assert-True ($runNoComments -notmatch $ifKeyPattern) 'the run step must stay unconditionally gated on success'
    Assert-True ($runNoComments -notmatch '\|\|\s*true\b') 'the run step must not swallow failures with || true'
    Assert-True ($runNoComments -notmatch '\|\|\s*exit\s+0\b') 'the run step must not swallow failures with || exit 0'
    Assert-True ($runNoComments -notmatch '(?m)^\s*set\s+\+e\b') 'the run step must not disable set -e'
    Assert-True ($runNoComments -notmatch '(?m)^\s*set\s+\+o\s+errexit\b') 'the run step must not disable errexit'
}

Test-Case 'ignored job publishes a per-crate report with the real inventory' {
    Assert-True ($reportStep.Length -gt 0) 'Report the ignored tests per crate step not found'
    Assert-True ($reportNoComments -match 'cargo nextest list\b') 'the report must come from nextest, not a hardcoded count'
    Assert-True ($reportNoComments -match '--run-ignored\s+only') 'the report must list the ignored tests'
    Assert-True ($reportNoComments -match '(?m)^[ \t]*\|[ \t]*select\(\s*\.value\.ignored\s*\)[ \t]*\r?$') 'the jq filter must keep only ignored testcases'
    Assert-True ($reportNoComments -match '\$GITHUB_STEP_SUMMARY') 'the report must show up in the job summary'
    Assert-True ($reportNoComments -match '(?m)^[ \t]*for crate in lc-media lc-server xtask; do[ \t]*\r?$') 'the report must count each of the three crates'
    Assert-True ($reportNoComments -match 'total=\$\(\(total \+ count\)\)') 'the report must sum the per-crate counts'
    Assert-True ($reportNoComments -match '\| \*\*total\*\* \| \*\*\$total\*\* \|') 'the report must publish its total'
    foreach ($crate in @('lc-media', 'lc-server', 'xtask')) {
        Assert-True ($reportNoComments -match "(?m)\s-p\s+$([regex]::Escape($crate))\b") "the report list must select $crate"
    }

    $counts = @{ 'lc-media' = 0; 'lc-server' = 0; 'xtask' = 0 }
    $outside = @()
    $ignoredLines = @(& git -C $script:RepoRoot grep -n '^\s*#\[ignore' -- '*.rs' 2>$null)
    Assert-True ($ignoredLines.Count -gt 0) 'git grep found no #[ignore] attributes'
    foreach ($line in $ignoredLines) {
        $path = ($line -split ':', 2)[0]
        if ($path -like 'crates/lc-media/*') { $counts['lc-media'] = $counts['lc-media'] + 1 }
        elseif ($path -like 'apps/lc-server/*') { $counts['lc-server'] = $counts['lc-server'] + 1 }
        elseif ($path -like 'xtask/*') { $counts['xtask'] = $counts['xtask'] + 1 }
        else { $outside += $path }
    }
    Assert-Equal 0 $outside.Count ("ignored tests outside the three nightly crates: " + (($outside | Select-Object -Unique) -join ', '))
    Assert-Equal 39 $counts['lc-media'] 'lc-media ignored-test inventory changed; update the documented counts'
    Assert-Equal 11 $counts['lc-server'] 'lc-server ignored-test inventory changed; update the documented counts'
    Assert-Equal 2 $counts['xtask'] 'xtask ignored-test inventory changed; update the documented counts'
    $total = ($counts.Values | Measure-Object -Sum).Sum
    Assert-Equal $ignoredLines.Count $total 'every #[ignore] line must be counted by one of the three crates'
    Assert-Equal 52 $total 'the report total must be 52 (39+11+2), not just a table'
}

Test-Case 'ignored job uploads the report even when a test fails' {
    Assert-True ($uploadStep.Length -gt 0) 'Upload the ignored test report step not found'
    Assert-True ($uploadNoComments -match '(?m)^\s+name:\s*ignored-tests-report[ \t]*\r?$') 'the job must upload the report as an artifact'
    Assert-True ($uploadNoComments -match '(?m)^\s+if-no-files-found:\s*error[ \t]*\r?$') 'the artifact must fail when it is empty'
    $uploadIf = [regex]::Match($uploadNoComments, '(?m)^[ \t]+["'']?if["'']?\s*:\s*([^\r\n]*)').Groups[1].Value
    $uploadIf = $uploadIf.Trim()
    $uploadIf = $uploadIf -replace '^\$\{\{\s*', ''
    $uploadIf = $uploadIf -replace '\s*\}\}$', ''
    $uploadIf = $uploadIf.Trim().Trim('"', "'")
    Assert-Equal 'always()' $uploadIf 'the report artifact must upload even when the tests fail'
    foreach ($artifactPath in @('target/ignored-tests.json', 'target/ignored-tests.md', 'target/ignored-tests.txt')) {
        Assert-True ($uploadNoComments -match [regex]::Escape($artifactPath)) "the artifact must include $artifactPath"
    }
}
