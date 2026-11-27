# Guards for the CI layout that decides pull request latency.
#
# Protected here:
# - the advisory Bombadil exploration runs as its own `bombadil` job, not as two
#   steps of `test (ubuntu-latest)`: inside the test job its ~3 minute run sat on
#   the critical path of every Rust PR while the rest of the Linux job finished
#   in ~4.5 minutes;
# - `ci-gate` keeps a stability window measured in seconds: the window only
#   catches a check run created a few seconds after the last one, so a full
#   minute there adds a minute to every PR before the required check turns green.
# - Bombadil stays advisory: moving it to its own job must not make it blocking
#   (its traces stay uploaded, the check stays green).

function Get-JobBlock {
    param([string]$Workflow, [string]$JobId)
    $match = [regex]::Match($Workflow, "(?ms)^  $([regex]::Escape($JobId)):[ \t]*\r?\n(.*?)(?=^  [A-Za-z_][A-Za-z0-9_-]*:[ \t]*(?:#.*)?\r?$|\z)")
    return $match.Groups[1].Value
}

# Every job of a workflow, as id + block. Job ids are the only keys indented by
# exactly two spaces inside `jobs:` (the `on:`/`env:` keys above it are keys of
# their own blocks), so the scan starts at `jobs:`.
function Get-WorkflowJobs {
    param([string]$Workflow)
    $jobsAt = [regex]::Match($Workflow, '(?m)^jobs:[ \t]*\r?$')
    if (-not $jobsAt.Success) { return @() }
    $tail = $Workflow.Substring($jobsAt.Index + $jobsAt.Length)
    $jobs = foreach ($m in [regex]::Matches($tail, '(?m)^  ([A-Za-z_][A-Za-z0-9_-]*):[ \t]*(?:#.*)?\r?\n')) {
        [pscustomobject]@{ Id = $m.Groups[1].Value; Body = (Get-JobBlock $tail $m.Groups[1].Value) }
    }
    return @($jobs)
}

function Get-GateSeconds {
    param([string]$Workflow, [string]$Name)
    $match = [regex]::Match($Workflow, "(?m)^\s+$([regex]::Escape($Name)):\s*`"(\d+)`"")
    if ($match.Groups[1].Value -eq '') { throw "missing $Name in ci-gate.yml" }
    return [int]$match.Groups[1].Value
}

$script:RustWorkflowPath = Join-Path $script:RepoRoot '.github/workflows/rust.yml'
$script:GateWorkflowPath = Join-Path $script:RepoRoot '.github/workflows/ci-gate.yml'
$script:RustWorkflow = Get-Content -Raw $script:RustWorkflowPath
$script:GateWorkflow = Get-Content -Raw $script:GateWorkflowPath

Test-Case 'the advisory Bombadil exploration is its own job' {
    $bombadil = Get-JobBlock $script:RustWorkflow 'bombadil'
    Assert-True ($bombadil.Length -gt 0) 'the bombadil job is missing from rust.yml'
    Assert-True ($bombadil -match '(?m)^    name: bombadil') 'the bombadil job must keep an explicit display name'
    $explore = [regex]::Match($bombadil, '(?ms)^      - name: [^\r\n]*overlay board[^\r\n]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
    Assert-True ($explore.Length -gt 0) 'the Bombadil exploration step is missing from the bombadil job'
    Assert-True ($explore -match '(?m)^        continue-on-error: true[ \t]*\r?$') 'the exploration must stay advisory (continue-on-error)'
    Assert-True ($explore.Contains('BOMBADIL_TIME_LIMIT: 3m')) 'the exploration must keep its 3 minute budget'
    Assert-True ($explore.Contains('LC_SERVER_BIN: ${{ github.workspace }}/target/debug/lc-server')) 'the exploration must still drive the real lc-server'
    Assert-True ($explore.Contains('npm run admin')) 'the exploration must still cover the admin console'
    Assert-True ($explore.Contains('npm run board')) 'the exploration must still cover the overlay board'
    Assert-True ($bombadil.Contains('cargo build -p lc-server --bin lc-server --locked')) 'the bombadil job must build the server it explores'
    Assert-True ($bombadil.Contains('path: tools/bombadil/out/')) 'the bombadil job must upload the exploration traces'
    Assert-True ($bombadil -match 'actions/upload-artifact@[0-9a-f]{40}') 'the artifact action must stay pinned to a commit'

    # The whole point of the split: the advisory run no longer holds the job
    # that also runs the suite and the console E2E. Comments are inert (the job
    # block's trailing comment describes the next job), so drop them first.
    $testJob = Get-JobBlock $script:RustWorkflow 'test'
    Assert-True ($testJob.Length -gt 0) 'the test job is missing from rust.yml'
    $testSteps = (($testJob -split "\r?\n") | Where-Object { $_ -notmatch '^\s*#' }) -join "`n"
    Assert-True ($testSteps -notmatch '(?i)bombadil') 'the test job must not run Bombadil: it belongs to the bombadil job'
}

Test-Case 'the aggregated gate keeps a short stability window' {
    $timeout = Get-GateSeconds $script:GateWorkflow 'GATE_TIMEOUT_SECONDS'
    $poll = Get-GateSeconds $script:GateWorkflow 'GATE_POLL_SECONDS'
    $stability = Get-GateSeconds $script:GateWorkflow 'GATE_STABILITY_SECONDS'
    Assert-Equal 3300 $timeout 'the gate must keep its 55 minute budget (ci-gate timeout-minutes is 60)'
    Assert-True ($poll -ge 5) "GATE_POLL_SECONDS=$poll hammers the checks API"
    Assert-True ($stability -le 30) "GATE_STABILITY_SECONDS=$stability adds more than 30 s to every PR before ci-gate turns green"
    Assert-True ($stability -ge (2 * $poll)) "GATE_STABILITY_SECONDS=$stability must span at least two poll cycles (GATE_POLL_SECONDS=$poll)"
}

Test-Case 'no pull request job starts only after another one' {
    # Why this matters: ci-gate turns green once every check run it can see is
    # completed AND the set has been identical for GATE_STABILITY_SECONDS (20 s,
    # two polls). GitHub creates a job's check run with its workflow, except for
    # a job with `needs`: that one only appears when its dependency finishes, so
    # it can surface long after ci-gate already concluded, and the gate would
    # have let the pull request merge without it. Every `needs` of a workflow
    # that runs on `pull_request` must therefore be gated to another event.
    $workflows = @(Get-ChildItem (Join-Path $script:RepoRoot '.github/workflows') -Filter *.yml)
    Assert-True ($workflows.Count -ge 8) "only $($workflows.Count) workflows were found: the layout guard inspects nothing"
    $inspected = 0
    foreach ($file in $workflows) {
        $text = Get-Content -Raw $file.FullName
        if ($text -notmatch '(?m)^  pull_request:') { continue }
        $jobs = Get-WorkflowJobs $text
        # `@()` because a single job comes back as a bare object, not an array.
        Assert-True ((@($jobs).Count) -gt 0) "$($file.Name) declares pull_request but no job could be parsed"
        foreach ($job in $jobs) {
            $inspected++
            if ($job.Body -notmatch '(?m)^    needs:') { continue }
            # Only the job level `if:` counts: steps may legitimately mention
            # `refs/tags/` or the event name while the job itself runs on PRs.
            $jobIf = [regex]::Match($job.Body, '(?ms)^    if:[ \t]*(.*?)(?=^    [A-Za-z_][A-Za-z0-9_-]*:[ \t]|\z)').Groups[1].Value
            $otherEvent = $jobIf -match "github\.event_name\s*!=\s*'pull_request'" -or $jobIf -match 'refs/tags/|github\.ref'
            Assert-True $otherEvent "$($file.Name): job '$($job.Id)' has needs but can run on pull_request, so its check run appears after the others and ci-gate's 20 s stability window can miss it"
        }
    }
    Assert-True ($inspected -ge 15) "only $inspected pull request jobs were inspected"
}
