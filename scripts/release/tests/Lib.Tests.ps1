# Version model, consistency check, semver and last-tag lookup.

Test-Case 'Reads versions of the real repo' {
    $members = Get-ServerWorkspaceMembers -Root $script:RepoRoot
    Assert-True ($members -contains 'lc-server') 'lc-server is a server member'
    Assert-True ($members -contains 'xtask') 'xtask is a server member'
    Assert-True (-not ($members -contains 'lc-overlay')) 'lc-overlay is not a server member'

    $server = @(Get-ComponentVersionSources -Root $script:RepoRoot -Component server)
    Assert-Equal 1 @($server | Where-Object { $_.File -eq 'Cargo.toml' }).Count 'Cargo.toml sources'
    Assert-Equal $members.Count @($server | Where-Object { $_.File -eq 'Cargo.lock' }).Count 'Cargo.lock sources'
    Assert-True (@($server | Where-Object { $_.Label -eq 'Cargo.lock lc-server' }).Count -eq 1) 'lc-server lock label'
    # fuzz/Cargo.lock is its own workspace: it only repeats the version of the
    # server crates the fuzz targets link.
    Assert-Equal 3 @($server | Where-Object { $_.File -eq 'fuzz/Cargo.lock' }).Count 'fuzz lock sources'
    Assert-True (@($server | Where-Object { $_.Label -eq 'fuzz/Cargo.lock lc-media' }).Count -eq 1) 'lc-media fuzz lock label'

    Assert-Equal 3 @(Get-ComponentVersionSources -Root $script:RepoRoot -Component overlay).Count 'overlay sources'
    Assert-Equal 3 @(Get-ComponentVersionSources -Root $script:RepoRoot -Component extension).Count 'extension sources'
}

Test-Case 'Consistent repo has no errors' {
    $repo = New-TestRepo
    Assert-Equal 0 @(Test-VersionConsistency -Root $repo).Count 'errors'
}

Test-Case 'Detects a diverging file' {
    $repo = New-TestRepo
    # extension/package.json is the version source, so the lockfile is the file
    # that can diverge from it.
    $file = Join-Path $repo 'extension/package-lock.json'
    $text = [IO.File]::ReadAllText($file)
    [IO.File]::WriteAllText($file, ([regex]'"version": "[^"]+"').Replace($text, '"version": "9.9.9"', 1))
    $errors = @(Test-VersionConsistency -Root $repo)
    Assert-Equal 1 $errors.Count 'errors'
    Assert-True ($errors[0] -like '*extension/package-lock.json*') "message names the file: $($errors[0])"
    Assert-True ($errors[0] -like '*9.9.9*') "message names the version: $($errors[0])"
}

Test-Case 'Detects a stale fuzz lockfile' {
    # fuzz/ has its own lockfile: a server bump that forgets it leaves
    # `cargo test --manifest-path fuzz/Cargo.toml --locked` failing in the nightly.
    $repo = New-TestRepo
    $path = Join-Path $repo 'fuzz/Cargo.lock'
    $text = [IO.File]::ReadAllText($path)
    $stale = [regex]::Replace($text, '(?m)^(name = "lc-media"\r?\nversion = ")[^"]+(")', '${1}0.0.0${2}')
    [IO.File]::WriteAllText($path, $stale)
    $errors = @(Test-VersionConsistency -Root $repo)
    Assert-Equal 1 $errors.Count 'errors'
    Assert-True ($errors[0] -like '*fuzz/Cargo.lock*') "message names the file: $($errors[0])"
    Assert-True ($errors[0] -like '*0.0.0*') "message names the version: $($errors[0])"
}

Test-Case 'Semver compares numerically' {
    Assert-Equal 1 (Compare-SemVer -A '3.10.0' -B '3.9.0') '3.10.0 vs 3.9.0'
    Assert-Equal 0 (Compare-SemVer -A '3.5.1' -B '3.5.1') '3.5.1 vs 3.5.1'
    Assert-Equal -1 (Compare-SemVer -A '0.1.0' -B '0.2.0') '0.1.0 vs 0.2.0'
}

Test-Case 'Last tag ignores prereleases' {
    $repo = New-TestRepo
    foreach ($t in 'server-v3.1.0-beta.3', 'server-v3.9.0', 'server-v3.10.0', 'legacy-ts-2.0.2', 'extension-latest') {
        Invoke-TestGit $repo tag $t | Out-Null
    }
    $last = Get-LastReleaseTag -Root $repo -Component server
    Assert-Equal 'server-v3.10.0' $last.Tag 'tag'
    Assert-Equal '3.10.0' $last.Version 'version'
    Assert-Equal $null (Get-LastReleaseTag -Root $repo -Component extension) 'extension'
}
