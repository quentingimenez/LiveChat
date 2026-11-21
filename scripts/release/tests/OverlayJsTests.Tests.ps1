# Guard for Q-T1: the overlay JavaScript tests must run in CI, not only by hand.
# Protected here: the overlay-tauri workflow has a dedicated job that runs
# `node --test apps/lc-overlay/dev/tests/*.test.mjs` on overlay changes.

$workflowPath = Join-Path $script:RepoRoot '.github/workflows/overlay-tauri.yml'
$workflow = Get-Content -Raw $workflowPath

Test-Case 'overlay workflow runs the overlay JS tests in CI' {
    $job = [regex]::Match(
        $workflow,
        '(?ms)^  js-tests:\s*\r?\n(.*?)(?=^  installer:)'
    ).Groups[1].Value
    Assert-True ($job.Length -gt 0) 'js-tests job not found before the installer job'

    Assert-True ($job -match 'node --test apps/lc-overlay/dev/tests/\*\.test\.mjs') `
        'js-tests must run every overlay test file'
    Assert-True ($job -match 'actions/setup-node@[0-9a-f]{40}') `
        'js-tests must use a SHA-pinned setup-node action'
    Assert-True ($job -match '(?m)^\s+node-version:\s*[''"]?24[''"]?\s*$') `
        'js-tests must pin Node 24'
}
