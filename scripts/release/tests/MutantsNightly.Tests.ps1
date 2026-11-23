# Guard for Q-T4: the nightly mutation matrix must cover the critical crates
# beyond the original three scopes, and every matrix leg must stay coherent with
# scripts/mutants-baseline.json. A scope removed from one side, a provisional
# scope without its explicit fail-closed status, an unmeasured score recorded as
# measured, or a mismatch between the matrix's `provisional` flag and the
# baseline's `status` all fail here.
#
# The three Q-T4 scopes are provisional until the first nightly has produced
# their outcomes.json. Their `caught_pct` is null on purpose: the run step is
# skipped on pull_request and the nightly fails explicitly after uploading the
# report, instead of inventing a score. When a score is measured, update the
# baseline and the matrix flag in the same change, then update the pinned
# expectations below (they are deliberately exact so a silent recalibration
# cannot pass).

function ConvertFrom-MutantsScalar {
    param([string]$Raw)
    $value = $Raw.Trim()
    if ($value -eq "''" -or $value -eq '""') { return '' }
    if ($value.Length -ge 2) {
        $first = $value.Substring(0, 1)
        $last = $value.Substring($value.Length - 1, 1)
        if (($first -eq "'" -and $last -eq "'") -or ($first -eq '"' -and $last -eq '"')) {
            return $value.Substring(1, $value.Length - 2)
        }
    }
    return $value
}

function Get-MutantsJob {
    param([string]$Workflow)
    $match = [regex]::Match($Workflow, '(?ms)^  mutants:[ \t]*\r?\n(.*?)(?=^  [A-Za-z_][A-Za-z0-9_-]*:[ \t]*(?:#.*)?\r?$|\z)')
    return $match.Groups[1].Value
}

function Get-MutantsMatrix {
    param([string]$Job)
    $entries = @()
    $current = $null
    $inInclude = $false
    foreach ($rawLine in ($Job -split "\r?\n")) {
        if ($rawLine -match '^\s*#') { continue }
        $line = [regex]::Replace($rawLine, '\s+#.*$', '')
        if (-not $inInclude) {
            if ($line -match '^\s{8}include:\s*$') { $inInclude = $true }
            continue
        }
        if ($line -match '^\s*$') { continue }
        # The include list is indented 10 spaces; a dedent ends it (steps:,
        # timeout-minutes:, etc.).
        if ($line -notmatch '^\s{10,}\S') { break }
        if ($line -match '^\s{10,}-\s+scope:\s*(.+?)\s*$') {
            if ($null -ne $current) { $entries += $current }
            $current = [pscustomobject]@{
                scope = ConvertFrom-MutantsScalar $Matches[1]
                package = ''
                file = ''
                filter = ''
                provisional = $false
            }
            continue
        }
        if (($null -ne $current) -and ($line -match '^\s+([A-Za-z_][A-Za-z0-9_]*):\s*(.*?)\s*$')) {
            $key = $Matches[1]
            $value = ConvertFrom-MutantsScalar $Matches[2]
            if ($key -eq 'provisional') {
                if ($value -eq 'true') { $current.provisional = $true }
                elseif ($value -eq 'false') { $current.provisional = $false }
                else { throw "matrix provisional must be true or false, got '$value'" }
            }
            elseif ($key -eq 'package') { $current.package = $value }
            elseif ($key -eq 'file') { $current.file = $value }
            elseif ($key -eq 'filter') { $current.filter = $value }
        }
    }
    if ($null -ne $current) { $entries += $current }
    return $entries
}

function Get-MutantsNormalizedExpression {
    param([string]$Value)
    return (($Value -replace '\s+', ' ').Trim())
}

function Assert-MutantsCoherence {
    param($Matrix, $Baseline)
    $matrixScopes = @($Matrix | ForEach-Object { $_.scope })
    $baselineScopes = @($Baseline.PSObject.Properties | ForEach-Object { $_.Name })
    foreach ($scope in $matrixScopes) {
        if ($baselineScopes -notcontains $scope) {
            throw "scope '$scope' is in the workflow matrix but not in scripts/mutants-baseline.json"
        }
    }
    foreach ($scope in $baselineScopes) {
        if ($matrixScopes -notcontains $scope) {
            throw "scope '$scope' is in the baseline but not in the workflow matrix"
        }
    }
    Assert-Equal $matrixScopes.Count $baselineScopes.Count 'the workflow matrix and the baseline must list the same number of scopes'
    foreach ($entry in $Matrix) {
        $row = $Baseline.($entry.scope)
        if ($null -eq $row) { throw "missing baseline row for '$($entry.scope)'" }
        if (($row.status -ne 'measured') -and ($row.status -ne 'provisional')) {
            throw "scope '$($entry.scope)' has status '$($row.status)' (expected measured or provisional)"
        }
        $isProvisional = ($row.status -eq 'provisional')
        if ($isProvisional -ne [bool]$entry.provisional) {
            throw "scope '$($entry.scope)' matrix provisional flag and baseline status disagree"
        }
        if ($isProvisional) {
            if ($null -ne $row.caught_pct) {
                throw "provisional scope '$($entry.scope)' must keep caught_pct null until measured"
            }
        }
        else {
            if (-not (($row.caught_pct -is [int]) -or ($row.caught_pct -is [long]))) {
                throw "measured scope '$($entry.scope)' caught_pct must be an integer"
            }
            if (($row.caught_pct -lt 0) -or ($row.caught_pct -gt 100)) {
                throw "measured scope '$($entry.scope)' caught_pct must be between 0 and 100"
            }
        }
        if (-not (($row.min_mutants -is [int]) -or ($row.min_mutants -is [long]))) {
            throw "scope '$($entry.scope)' min_mutants must be an integer"
        }
        if ($row.min_mutants -lt 1) {
            throw "scope '$($entry.scope)' min_mutants must be at least 1"
        }
    }
}

$workflowPath = Join-Path $script:RepoRoot '.github/workflows/verification-nightly.yml'
$workflow = Get-Content -Raw $workflowPath
$job = Get-MutantsJob $workflow
$matrix = @(Get-MutantsMatrix $job)

$expected = @(
    [pscustomobject]@{ scope = 'lc-protocol'; package = 'lc-protocol'; file = ''; filter = ''; provisional = $false; caught = 99; min = 47; status = 'measured' }
    [pscustomobject]@{ scope = 'lc-media-guard'; package = 'lc-media'; file = 'crates/lc-media/src/guard.rs'; filter = 'kind(lib) | binary(ssrf_guard) | binary(properties)'; provisional = $false; caught = 99; min = 86; status = 'measured' }
    [pscustomobject]@{ scope = 'lc-db-queue'; package = 'lc-db'; file = 'crates/lc-db/src/repos/playback_jobs.rs'; filter = ''; provisional = $false; caught = 93; min = 92; status = 'measured' }
    [pscustomobject]@{ scope = 'lc-playback-payload'; package = 'lc-playback'; file = 'crates/lc-playback/src/payload.rs'; filter = 'kind(lib) | binary(engine_scenarios)'; provisional = $true; caught = $null; min = 44; status = 'provisional' }
    [pscustomobject]@{ scope = 'lc-db-core'; package = 'lc-db'; file = 'crates/lc-db/src/db.rs'; filter = ''; provisional = $true; caught = $null; min = 77; status = 'provisional' }
    [pscustomobject]@{ scope = 'lc-media-storage'; package = 'lc-media'; file = 'crates/lc-media/src/storage.rs'; filter = 'kind(lib) | binary(properties)'; provisional = $true; caught = $null; min = 118; status = 'provisional' }
)

Test-Case 'mutation matrix covers the six critical scopes' {
    Assert-True ($job.Length -gt 0) 'mutants job not found in verification-nightly.yml'
    Assert-Equal 6 $matrix.Count 'the mutants matrix must contain exactly six scopes'
    $seen = @{}
    foreach ($entry in $matrix) {
        if ($seen.ContainsKey($entry.scope)) { throw "duplicate mutation scope '$($entry.scope)'" }
        $seen[$entry.scope] = $true
    }
    foreach ($want in $expected) {
        $scopeMatches = @($matrix | Where-Object { $_.scope -eq $want.scope })
        Assert-Equal 1 $scopeMatches.Count "matrix must contain exactly one '$($want.scope)' leg"
        $got = $scopeMatches[0]
        Assert-Equal $want.package $got.package "scope '$($want.scope)' package changed"
        Assert-Equal $want.file $got.file "scope '$($want.scope)' file changed"
        Assert-Equal $want.filter $got.filter "scope '$($want.scope)' test filter changed"
        Assert-Equal $want.provisional $got.provisional "scope '$($want.scope)' provisional flag changed"
        $crateManifest = Join-Path (Join-Path (Join-Path $script:RepoRoot 'crates') $want.package) 'Cargo.toml'
        Assert-True (Test-Path $crateManifest) "scope '$($want.scope)' package '$($want.package)' has no crates/<package>/Cargo.toml"
        if ($want.file.Length -gt 0) {
            Assert-True (Test-Path (Join-Path $script:RepoRoot $want.file)) "scope '$($want.scope)' file '$($want.file)' does not exist"
            Assert-True ($want.file -like "crates/$($want.package)/src/*.rs") "scope '$($want.scope)' file must belong to its package's src/"
        }
        if ($want.filter.Length -gt 0) {
            Assert-True ($want.filter -match '^kind\(lib\)') "scope '$($want.scope)' filter must at least select the crate lib tests"
        }
    }
}

Test-Case 'mutation matrix is parallel, bounded, and refuses provisional legs on pull requests' {
    Assert-True ($job -match '(?m)^    timeout-minutes: 60[ \t]*\r?$') 'each mutation leg must keep its 60 minute timeout'
    Assert-True ($job -match '(?m)^      fail-fast: false[ \t]*\r?$') 'one failing scope must not cancel the other reports'
    Assert-True ($job -notmatch 'continue-on-error') 'the mutation job must not neutralise a failing leg'

    $runStep = [regex]::Match($job, '(?ms)^      - name: Run the .* mutation scope[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
    Assert-True ($runStep.Length -gt 0) 'the per-scope mutation run step was not found'
    $runIf = [regex]::Match($runStep, '(?m)^        if:\s*(.+?)\s*$').Groups[1].Value
    Assert-Equal "github.event_name != 'pull_request' || matrix.provisional != true" (Get-MutantsNormalizedExpression $runIf) 'provisional matrix legs must be skipped on pull_request'
    Assert-True ($runStep -match '(?m)^          SCOPE:\s*\$\{\{ matrix\.scope \}\}') 'the run step must receive the matrix scope'
    Assert-True ($runStep -match '(?m)^          PACKAGE:\s*\$\{\{ matrix\.package \}\}') 'the run step must receive the matrix package'
    Assert-True ($runStep -match '(?m)^          MUTATION_FILE:\s*\$\{\{ matrix\.file \}\}') 'the run step must receive the matrix file'
    Assert-True ($runStep -match '(?m)^          TEST_FILTER:\s*\$\{\{ matrix\.filter \}\}') 'the run step must receive the matrix filter'
    Assert-True ($runStep -notmatch '\|\|\s*true\b') 'the run step must not swallow failures with || true'
    Assert-True ($runStep -notmatch 'set\s+\+e[ \t]*\r?\n\s*#?\s*echo') 'the run step must not disable errexit around the comparison'
}

Test-Case 'mutation run constructs each cargo-mutants command and fails closed' {
    $runStep = [regex]::Match($job, '(?ms)^      - name: Run the .* mutation scope[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
    $run = [regex]::Match($runStep, '(?ms)^        run: \|[ \t]*\r?\n(.*)\z').Groups[1].Value
    Assert-True ($run.Length -gt 0) 'the mutation run scalar was not found'

    foreach ($snippet in @(
        'args=(cargo mutants --output "$output" -p "$PACKAGE")'
        'if [ -n "$MUTATION_FILE" ]; then'
        'args+=(--file "$MUTATION_FILE")'
        'args+=(--test-tool nextest -j 2)'
        'if [ -n "$TEST_FILTER" ]; then'
        'args+=(-- -E "$TEST_FILTER")'
        'rm -rf "$output"'
        'mkdir -p "$output"'
        'if [ "$rc" -ne 0 ] && [ "$rc" -ne 2 ]; then'
        'actual=$(jq -r'
        'total=$(jq -r'
        'failures=$(jq -r'
        'status=$(jq -r'
        '.[$s].status // "measured"'
        '"$status" = "provisional"'
        '"$status" != "measured"'
        '"$failures" -gt 0'
        '"$total" -lt "$min_mutants"'
        '"$actual" -lt "$baseline"'
        'exit "$FAILED"'
    )) {
        Assert-True ($run.Contains($snippet)) "the run step must keep the exact shell fragment: $snippet"
    }
    Assert-Equal 1 ([regex]::Matches($run, '(?m)^\s*args=\(cargo mutants').Count) 'the run step must invoke cargo mutants exactly once'
    Assert-True ($run.Contains('outcomes.json')) 'the run step must read outcomes.json'
    Assert-True ($run.Contains('produced no outcomes.json')) 'a missing outcomes.json must fail the leg'
    Assert-True ($run.Contains('baseline is provisional')) 'a provisional baseline must fail with an actionable error'
    Assert-True ($run.Contains('status=measured')) 'the provisional error must say how to record the score'
    Assert-True ($run.Contains('provisional=false')) 'the provisional error must say how to re-enable the leg'
    Assert-True ($run.Contains('unclassified Failure')) 'Failure outcomes must fail the leg'
    Assert-True ($run.Contains('dropped below the baseline')) 'a score below the baseline must fail the leg'
}

Test-Case 'baseline matches the mutation matrix and its measurement state' {
    $baselinePath = Join-Path $script:RepoRoot 'scripts/mutants-baseline.json'
    Assert-True (Test-Path $baselinePath) 'scripts/mutants-baseline.json is missing'
    $baseline = Get-Content -Raw $baselinePath | ConvertFrom-Json
    Assert-MutantsCoherence $matrix $baseline
    foreach ($want in $expected) {
        $row = $baseline.($want.scope)
        Assert-Equal $want.status $row.status "scope '$($want.scope)' baseline status changed"
        Assert-Equal $want.min $row.min_mutants "scope '$($want.scope)' min_mutants changed; recalibrate the baseline, the matrix, and this guard together"
        if ($want.status -eq 'provisional') {
            Assert-True ($null -eq $row.caught_pct) "provisional scope '$($want.scope)' must keep caught_pct null until a real score is measured"
        }
        else {
            Assert-Equal $want.caught $row.caught_pct "scope '$($want.scope)' caught_pct changed"
        }
    }
}

Test-Case 'baseline and workflow incoherence is rejected' {
    $baselinePath = Join-Path $script:RepoRoot 'scripts/mutants-baseline.json'
    $baseline = Get-Content -Raw $baselinePath | ConvertFrom-Json

    $missing = $baseline | ConvertTo-Json -Depth 5 | ConvertFrom-Json
    $missing.PSObject.Properties.Remove('lc-db-core')
    Assert-Throws { Assert-MutantsCoherence $matrix $missing } 'not in scripts/mutants-baseline.json'

    $orphan = $baseline | ConvertTo-Json -Depth 5 | ConvertFrom-Json
    $orphan | Add-Member -NotePropertyName 'lc-db-orphan' -NotePropertyValue ([pscustomobject]@{ caught_pct = 99; min_mutants = 1; status = 'measured' })
    Assert-Throws { Assert-MutantsCoherence $matrix $orphan } 'not in the workflow matrix'

    $mismatch = $baseline | ConvertTo-Json -Depth 5 | ConvertFrom-Json
    $mismatch.'lc-db-core'.status = 'measured'
    Assert-Throws { Assert-MutantsCoherence $matrix $mismatch } 'matrix provisional flag and baseline status disagree'

    $unmeasured = $matrix | ForEach-Object {
        [pscustomobject]@{ scope = $_.scope; package = $_.package; file = $_.file; filter = $_.filter; provisional = $_.provisional }
    }
    ($unmeasured | Where-Object { $_.scope -eq 'lc-playback-payload' }).provisional = $false
    $measuredNull = $baseline | ConvertTo-Json -Depth 5 | ConvertFrom-Json
    $measuredNull.'lc-playback-payload'.status = 'measured'
    Assert-Throws { Assert-MutantsCoherence $unmeasured $measuredNull } 'caught_pct must be an integer'

    $numericProvisional = $baseline | ConvertTo-Json -Depth 5 | ConvertFrom-Json
    $numericProvisional.'lc-media-storage'.caught_pct = 100
    Assert-Throws { Assert-MutantsCoherence $matrix $numericProvisional } 'must keep caught_pct null'
}

Test-Case 'mutation reports are uploaded per scope even when a leg fails' {
    $uploadStep = [regex]::Match($job, '(?ms)^      - name: Upload the .* mutation report[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
    Assert-True ($uploadStep.Length -gt 0) 'the per-scope mutation report upload step was not found'
    $uploadIf = [regex]::Match($uploadStep, '(?m)^        if:\s*(.+?)\s*$').Groups[1].Value
    Assert-Equal "always() && (github.event_name != 'pull_request' || matrix.provisional != true)" (Get-MutantsNormalizedExpression $uploadIf) 'the report upload must run on failure, but not for skipped provisional legs'
    Assert-True ($uploadStep.Contains('name: mutants-report-${{ matrix.scope }}')) 'each scope must upload its own artifact'
    Assert-True ($uploadStep.Contains('path: target/mutants/${{ matrix.scope }}/mutants.out')) 'the artifact path must be the matrix scope report'
    Assert-True ($uploadStep -match '(?m)^          if-no-files-found: warn[ \t]*\r?$') 'a missing report must warn without hiding the real mutation failure'
    Assert-True ($uploadStep -match 'actions/upload-artifact@[0-9a-f]{40}') 'the artifact action must stay pinned to a commit'
}

Test-Case 'mutation matrix remains reachable from the nightly and pull_request triggers' {
    $onBlock = [regex]::Match($workflow, '(?ms)^on:[ \t]*\r?\n(.*?)(?=^[A-Za-z_][^\r\n:]*:[ \t]*(?:#.*)?\r?$|\z)').Groups[1].Value
    Assert-True ($onBlock -match "(?m)^\s*-\s*cron:\s*'30 3 \* \* \*'[ \t]*\r?$") 'the nightly cron must stay at 30 3 * * *'
    Assert-True ($onBlock -match '(?m)^  workflow_dispatch:[ \t]*\r?$') 'workflow_dispatch must stay available for a manual measurement run'
    Assert-True ($onBlock -match '(?m)^\s*-\s*"\.github/workflows/verification-nightly\.yml"[ \t]*\r?$') 'a PR changing the workflow must run it'
    Assert-True ($onBlock -match '(?m)^\s*-\s*"scripts/mutants-baseline\.json"[ \t]*\r?$') 'a PR changing the baseline must run it'
}
