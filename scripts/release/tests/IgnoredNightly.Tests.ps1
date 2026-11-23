# Guard for Q-T2: the `#[ignore]` tests run in the nightly workflow only.
# Protected here: verification-nightly.yml keeps its nightly schedule and manual
# dispatch, the `ignored` job is gated by the exact two-event expression and is
# unreachable from a pull_request. Its run step executes the exact nextest
# command with the filter selected from scripts/release/ignored-nightly-tests.json,
# with no shell escape hatch, and publishes the per-crate report. A
# commented-out command, a trailing `#`, an extra `;`/`&&`/`||`/`!`, a widened
# `if:`, a filter inlined in the run step, or an ignored test outside the three
# crates all fail here.
#
# The manifest classifies every `#[ignore]` test as `local` (runs automatically)
# or `network` (needs a third party service or real credentials, manual dispatch
# only). It must
# match the real inventory: a new, renamed or removed ignored test fails here
# until the manifest and the documented total (39+11+2=52) are updated. Network
# tests are not deleted: they stay listed in the report and run with
# workflow_dispatch `network_tests=true`.

$workflowPath = Join-Path $script:RepoRoot '.github/workflows/verification-nightly.yml'
$workflow = Get-Content -Raw $workflowPath
$job = [regex]::Match($workflow, '(?ms)^  ignored:[ \t]*\r?\n(.*?)(?=^  [A-Za-z_][A-Za-z0-9_-]*:[ \t]*(?:#.*)?\r?$|\z)').Groups[1].Value
$runStep = [regex]::Match($job, '(?ms)^      - name: Run the ignored tests[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
$runScalar = [regex]::Match($runStep, '(?ms)^        run: \|[ \t]*\r?\n(.*)\z').Groups[1].Value
$selectStep = [regex]::Match($job, '(?ms)^      - name: Select the ignored tests to run[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
$reportStep = [regex]::Match($job, '(?ms)^      - name: Report the ignored tests per crate[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value
$uploadStep = [regex]::Match($job, '(?ms)^      - name: Upload the ignored test report[ \t]*\r?\n(.*?)(?=^      - name:|\z)').Groups[1].Value

# YAML comments and shell comments are inert. Drop whole-line comments and
# trailing ones before matching, so a commented-out command, path or gate can
# never satisfy an assertion. This is deliberately applied to every scalar.
function Remove-Comments {
    param([string]$Text)
    $kept = foreach ($line in ($Text -split "\r?\n")) {
        if ($line -match '^\s*#') { continue }
        [regex]::Replace($line, '\s+#.*$', '')
    }
    return ($kept -join "`n")
}

# The run step must be one shell command, so normalise it to compare the exact
# command instead of looking for substrings that `echo` or `; true` could fake.
function Get-NormalizedRunCommand {
    param([string]$Text)
    $parts = foreach ($line in ($Text -split "\r?\n")) {
        if ($line -match '^\s*#') { continue }
        $line = [regex]::Replace($line, '\s+#.*$', '')
        $line = $line -replace '\\\s*$', ''
        $line.Trim()
    }
    $joined = ($parts -join ' ')
    return ($joined -replace '\s+', ' ').Trim()
}

$onBlock = [regex]::Match($workflow, '(?ms)^on:[ \t]*\r?\n(.*?)(?=^[A-Za-z_][^\r\n:]*:[ \t]*(?:#.*)?\r?$|\z)').Groups[1].Value
$onNoComments = Remove-Comments $onBlock
$jobNoComments = Remove-Comments $job
$runNoComments = Remove-Comments $runStep
$selectNoComments = Remove-Comments $selectStep
$reportNoComments = Remove-Comments $reportStep
$uploadNoComments = Remove-Comments $uploadStep
$manifestPath = Join-Path $script:RepoRoot 'scripts/release/ignored-nightly-tests.json'
$manifest = $null
$manifestError = 'file is missing'
if (Test-Path $manifestPath) {
    try {
        $manifest = Get-Content -Raw $manifestPath | ConvertFrom-Json
        $manifestError = $null
    } catch {
        $manifestError = $_.Exception.Message
    }
}
$ifKeyPattern = '(?m)^[ \t]*["'']?if["'']?\s*:'
$expectedGate = "github.event_name == 'schedule' || github.event_name == 'workflow_dispatch'"
$expectedCommand = 'cargo nextest run -p lc-media -p lc-server -p xtask --all-features --locked --run-ignored only --no-fail-fast -E "$NEXTEST_FILTER"'

# The manifest classifies tests by source file, so the guard maps a path back to
# the crate whose ignored tests the nightly job runs.
function Get-NightlyCrate {
    param([string]$Path)
    if ($Path -like 'crates/lc-media/*') { return 'lc-media' }
    if ($Path -like 'apps/lc-server/*') { return 'lc-server' }
    if ($Path -like 'xtask/*') { return 'xtask' }
    return ''
}

# Every `#[ignore]` in the tree, with the function it decorates. The attribute
# is the last one before the test, but scan a few lines so a stacked attribute
# cannot hide the function name.
function Get-IgnoredSourceTests {
    $result = @()
    $lines = @(& git -C $script:RepoRoot grep -nE '^[[:space:]]*#\[ignore[[:space:]]*(\]|=)' -- '*.rs' 2>$null)
    Assert-True ($lines.Count -gt 0) 'git grep found no #[ignore] attributes'
    foreach ($line in $lines) {
        $match = [regex]::Match($line, '^([^:]+):(\d+):')
        if (-not $match.Success) { continue }
        $file = $match.Groups[1].Value
        $lineNumber = [int]$match.Groups[2].Value
        $fileLines = [IO.File]::ReadAllLines((Join-Path $script:RepoRoot $file))
        $function = $null
        for ($index = $lineNumber; $index -lt [Math]::Min($lineNumber + 5, $fileLines.Count); $index++) {
            $trimmed = $fileLines[$index].Trim()
            if ($trimmed -match '^(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)') {
                $function = $Matches[1]
                break
            }
            if ($trimmed -notmatch '^(?:#\[|//|$)') { break }
        }
        Assert-True ($null -ne $function) "no fn after #[ignore] at ${file}:$lineNumber"
        $result += [pscustomobject]@{ File = $file; Function = $function }
    }
    return $result
}

Test-Case 'ignored job keeps the nightly schedule and manual dispatch trigger' {
    Assert-True ($onNoComments.Length -gt 0) 'the workflow on: block not found'
    Assert-True ($onNoComments -match '(?m)^  schedule:[ \t]*\r?$') 'verification-nightly.yml must keep the schedule trigger'
    Assert-True ($onNoComments -match "(?m)^\s*-\s*cron:\s*'30 3 \* \* \*'[ \t]*\r?$") 'the nightly cron must stay at 30 3 * * *'
    Assert-True ($onNoComments -match '(?m)^  workflow_dispatch:[ \t]*\r?$') 'verification-nightly.yml must keep workflow_dispatch'
}

Test-Case 'ignored job is named, bounded, and gated to the exact nightly events' {
    Assert-True ($job.Length -gt 0) 'ignored job not found in verification-nightly.yml'
    Assert-True ($jobNoComments -match '(?m)^    name: ignored tests[ \t]*\r?$') 'the job must stay named "ignored tests"'
    Assert-True ($jobNoComments -match '(?m)^    timeout-minutes: 60[ \t]*\r?$') 'the job must keep its 60 minute timeout'
    Assert-True ($jobNoComments -match '(?m)^      LC_ADMIN_WEB_BUILD:\s*"0"[ \t]*\r?$') 'the job must keep LC_ADMIN_WEB_BUILD=0'

    $ifLine = [regex]::Match($jobNoComments, '(?m)^    ["'']?if["'']?\s*:\s*([^\r\n]*)').Groups[1].Value
    Assert-True ($ifLine.Length -gt 0) 'the job must keep its if: gate'
    $ifValue = $ifLine.Trim()
    $ifValue = $ifValue -replace '^\$\{\{\s*', ''
    $ifValue = $ifValue -replace '\s*\}\}$', ''
    $ifValue = ($ifValue -replace '\s+', ' ').Trim()
    Assert-Equal $expectedGate $ifValue 'the job if: must be exactly the schedule and workflow_dispatch expression'
    Assert-True ($ifValue -notmatch 'pull_request') 'the job if: must not mention pull_request'
    Assert-True ($jobNoComments -notmatch '(?m)^    ["'']?(?:on|pull_request|push|schedule|workflow_dispatch|workflow_run|release|repository_dispatch|workflow_call)["'']?\s*:') 'the ignored job must not declare another trigger'
    Assert-True ($jobNoComments -notmatch '(?m)^\s*["'']?pull_request["'']?\s*:') 'the job must not declare a pull_request trigger'
    Assert-True ($jobNoComments -notmatch 'secrets\.') 'the job must not use any secret'
    Assert-True ($jobNoComments -notmatch 'continue-on-error') 'the job must not neutralise a failing step'
}

Test-Case 'ignored job runs the exact nextest command without a shell escape' {
    Assert-True ($runStep.Length -gt 0) 'Run the ignored tests step not found'
    Assert-True ($runScalar.Length -gt 0) 'the run step must use a literal run: | scalar'
    $runCommand = Get-NormalizedRunCommand $runScalar
    Assert-Equal $expectedCommand $runCommand 'the run step must execute the exact nextest command'
    Assert-True ($runCommand -notmatch '(?:^|\s)(?:;|&&|\|\||!|if|then|echo|exit|true)(?:\s|$)') 'the run command must not contain a shell escape token'
    Assert-Equal 1 ([regex]::Matches($jobNoComments, '(?m)^\s*cargo nextest run\b').Count) `
        'exactly one non-commented cargo nextest run is allowed in the job'
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
    Assert-True ($reportNoComments -match 'ignored-nightly-tests\.json') 'the report must read the network count from the manifest'
    Assert-True ($reportNoComments -match 'inputs\.network_tests') 'the report must follow the manual dispatch input'
    Assert-True ($reportNoComments -match 'run_count=\$total') 'the report must count every test when network_tests=true'
    Assert-True ($reportNoComments -match 'run_count=\$\(\(total - network\)\)') 'the report must subtract the network tests on the nightly schedule'
    Assert-True ($reportNoComments -match 'run_label="run without network"') 'the report must label the scheduled run'
    Assert-True ($reportNoComments -match 'run_label="run in this job \(network included\)"') 'the report must label the network dispatch run'
    Assert-True ($reportNoComments -match '\| \*\*\$run_label\*\* \| \*\*\$run_count\*\* \|') 'the report must publish the count actually run, not a fixed 34'
    foreach ($crate in @('lc-media', 'lc-server', 'xtask')) {
        Assert-True ($reportNoComments -match "(?m)\s-p\s+$([regex]::Escape($crate))\b") "the report list must select $crate"
    }
}

Test-Case 'ignored tests are classified in the manifest and match the real inventory' {
    Assert-True ($null -ne $manifest) ("the ignored-test manifest is missing or invalid: " + $manifestError)
    $entries = @($manifest.tests)
    Assert-Equal 52 $entries.Count 'the manifest must list every ignored test (39+11+2)'

    $counts = @{ 'lc-media' = 0; 'lc-server' = 0; 'xtask' = 0 }
    $outside = @()
    foreach ($entry in $entries) {
        $crate = Get-NightlyCrate $entry.file
        if ($crate) { $counts[$crate] = $counts[$crate] + 1 } else { $outside += $entry.file }
        Assert-True (-not [string]::IsNullOrWhiteSpace($entry.function)) "manifest entry without function: $($entry.file)"
        if ($entry.file -match '/tests/') {
            Assert-Equal $entry.function $entry.filter "an integration test filter is its test name: $($entry.file)"
            continue
        }
        # A unit test filter is `<module path>::tests::<fn>` (inline `mod tests`)
        # or `<module path>::<fn>` (a `*_tests.rs` module declared from its
        # parent). Check the module path against the file itself: checking only
        # the `::<fn>` suffix let a wrong module prefix slip through.
        $relative = $entry.file -replace '^(crates/[^/]+|apps/[^/]+|xtask)/src/', ''
        $segments = @($relative -replace '\.rs$', '' -split '/')
        if ($segments[-1] -in @('lib', 'main', 'mod')) {
            $segments = @($segments | Select-Object -First ($segments.Count - 1))
        }
        $modulePath = $segments -join '::'
        if ([string]::IsNullOrEmpty($modulePath)) {
            $rest = $entry.filter
        } else {
            Assert-True ($entry.filter.StartsWith("$modulePath::")) "a unit test filter must start with the module path of its file ($modulePath): $($entry.file)"
            $rest = $entry.filter.Substring($modulePath.Length + 2)
        }
        $nested = '^tests(::[A-Za-z0-9_]+)*::' + [regex]::Escape($entry.function) + '$'
        Assert-True ($rest -eq $entry.function -or $rest -match $nested) "a unit test filter must be <module>::tests::<fn> (or <module>::<fn> for a tests module): $($entry.file)::$($entry.function)"
    }
    Assert-Equal 0 $outside.Count ("ignored tests outside the three nightly crates: " + (($outside | Select-Object -Unique) -join ', '))
    Assert-Equal 39 $counts['lc-media'] 'lc-media ignored-test inventory changed; update the manifest and documented counts'
    Assert-Equal 11 $counts['lc-server'] 'lc-server ignored-test inventory changed; update the manifest and documented counts'
    Assert-Equal 2 $counts['xtask'] 'xtask ignored-test inventory changed; update the manifest and documented counts'

    $source = @(Get-IgnoredSourceTests)
    $sourcePairs = @($source | ForEach-Object { "$($_.File)|$($_.Function)" } | Sort-Object)
    $manifestPairs = @($entries | ForEach-Object { "$($_.file)|$($_.function)" } | Sort-Object)
    $missing = @($sourcePairs | Where-Object { $manifestPairs -notcontains $_ })
    $extra = @($manifestPairs | Where-Object { $sourcePairs -notcontains $_ })
    Assert-Equal 0 $missing.Count ("ignored tests missing from the manifest: " + ($missing -join ', '))
    Assert-Equal 0 $extra.Count ("manifest entries that are not #[ignore] tests: " + ($extra -join ', '))
    Assert-Equal $source.Count $entries.Count 'the manifest and the source inventory must have the same size'

    $filters = @($entries | ForEach-Object { $_.filter })
    Assert-Equal $filters.Count (@($filters | Select-Object -Unique)).Count 'two manifest entries must not share a nextest filter'

    $network = @($entries | Where-Object { $_.category -eq 'network' })
    $local = @($entries | Where-Object { $_.category -eq 'local' })
    Assert-Equal 19 $network.Count 'the network category must pin the 19 external-service or credential tests'
    Assert-Equal 33 $local.Count 'every other ignored test must stay in the automatic run'
    Assert-Equal 52 ($network.Count + $local.Count) 'each manifest entry must be local or network; no unclassified test'
    $tiktokNetwork = @('renders_a_carousel_comment', 'renders_the_shared_comment_from_the_user_link', 'resolves_the_short_link_to_a_comment_card')
    foreach ($entry in $entries) {
        if ($entry.file -eq 'crates/lc-media/tests/network.rs') {
            $expected = if ($entry.function -eq 'normalizes_generated_sample_with_ffmpeg') { 'local' } else { 'network' }
            Assert-Equal $expected $entry.category "$($entry.file)::$($entry.function) must stay $expected"
        } elseif ($entry.file -eq 'crates/lc-media/src/tiktok_comment.rs' -and $tiktokNetwork -contains $entry.function) {
            Assert-Equal 'network' $entry.category "the real-network TikTok comment test must stay network: $($entry.function)"
        } elseif ($entry.file -eq 'apps/lc-server/src/discord/runtime.rs' -and $entry.function -eq 'gateway_reel_configure_par_environnement') {
            Assert-Equal 'network' $entry.category 'the Discord gateway test needs real credentials: keep it network'
        } else {
            Assert-Equal 'local' $entry.category "only the network suites may be classified network: $($entry.file)::$($entry.function)"
        }
    }

    # A `network` test that returns when credentials are absent would give a
    # green manual dispatch without covering anything: pin its real behaviour.
    $discordPath = Join-Path $script:RepoRoot 'apps/lc-server/src/discord/runtime.rs'
    $discordSource = Get-Content -Raw $discordPath
    Assert-True ($discordSource -notmatch 'let Ok\(token\) = std::env::var\("DISCORD_TOKEN"\)') 'the Discord gateway test must fail without DISCORD_TOKEN, not return silently'
    Assert-True ($discordSource -match 'DISCORD_TOKEN absent') 'the Discord gateway test must explain why it cannot run without credentials'
    Assert-True ($discordSource -match 'discord\.com/api/v10/users/@me') 'the Discord gateway test must actually call the Discord API'

    Assert-True (-not [string]::IsNullOrWhiteSpace($manifest.network_filter)) 'the manifest must publish its network filter'
    Assert-True ($manifest.network_filter -notmatch '^\s*not\b') 'network_filter lists the network tests positively; the workflow applies the negation'
    $clauses = @($manifest.network_filter -split ' or ' | ForEach-Object { $_.Trim() })
    Assert-Equal $network.Count $clauses.Count 'the network filter must contain exactly one clause per network test'
    Assert-Equal $clauses.Count (@($clauses | Select-Object -Unique)).Count 'the network filter must not repeat a clause'
    foreach ($clause in $clauses) {
        Assert-True ($clause -match '^test\(=[A-Za-z0-9_:]+\)$') "unexpected network filter clause: $clause"
    }
    foreach ($entry in $network) {
        Assert-True ($clauses -contains "test(=$($entry.filter))") "the network filter misses $($entry.filter)"
    }
}

Test-Case 'ignored job selects the filter from the manifest and keeps network tests manual' {
    Assert-True ($selectStep.Length -gt 0) 'Select the ignored tests to run step not found'
    Assert-True ($selectNoComments -match 'network_tests') 'the select step must read the manual dispatch input'
    Assert-True ($selectNoComments -match "jq -r '\.network_filter'") 'the filter must come from the manifest, not a copy in the YAML'
    Assert-True ($selectNoComments -match 'not \(\$\(jq -r') 'the automatic run must exclude the network tests, not run them alone'
    Assert-True ($selectNoComments -match 'ignored-nightly-tests\.json') 'the select step must read the classification manifest'
    Assert-True ($selectNoComments -match 'all\(\)') 'the manual dispatch must be able to run every ignored test'
    Assert-True ($selectNoComments -match '\$GITHUB_ENV') 'the select step must export the filter to the next step'
    Assert-True ($selectNoComments -match 'NEXTEST_FILTER') 'the select step must name the exported filter'
    Assert-True ($runNoComments -match '-E\s+"\$NEXTEST_FILTER"') 'the run step must apply the selected filter'
    Assert-True ($runNoComments -notmatch 'test\(') 'the network filter must not be inlined in the run step'

    Assert-True ($onNoComments -match '(?m)^      network_tests:[ \t]*\r?$') 'workflow_dispatch must declare the network_tests input'
    Assert-True ($onNoComments -match '(?m)^        type:\s*boolean[ \t]*\r?$') 'network_tests must be a boolean input'
    Assert-True ($onNoComments -match '(?m)^        default:\s*false[ \t]*\r?$') 'network_tests must default to false so the scheduled run stays green'
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
