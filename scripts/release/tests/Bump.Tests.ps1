# bump: every file of a component, byte-faithful, refuses bad versions.

function Get-Versions {
    param([string]$Repo, [string]$Component)
    return @(Get-ComponentVersionSources -Root $Repo -Component $Component | ForEach-Object { $_.Version } | Sort-Object -Unique)
}

function Test-OnlyCrlf {
    param([byte[]]$Bytes)
    for ($i = 0; $i -lt $Bytes.Length; $i++) {
        if ($Bytes[$i] -eq 10 -and ($i -eq 0 -or $Bytes[$i - 1] -ne 13)) { return $false }
    }
    return $true
}

function Test-HasCrlf {
    param([byte[]]$Bytes)
    for ($i = 1; $i -lt $Bytes.Length; $i++) {
        if ($Bytes[$i] -eq 10 -and $Bytes[$i - 1] -eq 13) { return $true }
    }
    return $false
}

Test-Case 'Bump server writes every source' {
    $repo = New-TestRepo
    $overlayBefore = Get-ComponentVersion -Root $repo -Component overlay
    $written = @(Set-ComponentVersion -Root $repo -Component server -Version '9.0.0')
    Assert-True ($written -contains 'Cargo.toml') "written: $($written -join ', ')"
    Assert-True ($written -contains 'Cargo.lock') "written: $($written -join ', ')"
    $versions = @(Get-Versions $repo server)
    Assert-Equal 1 $versions.Count "server versions: $($versions -join ', ')"
    Assert-Equal '9.0.0' $versions[0] 'server version'
    Assert-Equal $overlayBefore (Get-ComponentVersion -Root $repo -Component overlay) 'overlay unchanged'
    Assert-Equal 0 @(Test-VersionConsistency -Root $repo).Count 'consistency'
}

Test-Case 'Bump keeps BOM and line endings' {
    $repo = New-TestRepo
    $files = 'Cargo.toml', 'Cargo.lock', 'apps/lc-overlay/tauri.conf.json', 'apps/lc-overlay/Cargo.toml'
    $before = @{}
    foreach ($f in $files) { $before[$f] = [IO.File]::ReadAllBytes((Join-Path $repo $f)) }
    $server = Get-ComponentVersion -Root $repo -Component server
    $overlay = Get-ComponentVersion -Root $repo -Component overlay
    $serverLockCount = @(Get-ServerWorkspaceMembers -Root $repo).Count
    Set-ComponentVersion -Root $repo -Component server -Version '10.0.0' | Out-Null
    Set-ComponentVersion -Root $repo -Component overlay -Version '10.0.0' | Out-Null

    $expectedDelta = @{
        'Cargo.toml'                     = ('10.0.0'.Length - $server.Length)
        'Cargo.lock'                     = $serverLockCount * ('10.0.0'.Length - $server.Length) + ('10.0.0'.Length - $overlay.Length)
        'apps/lc-overlay/tauri.conf.json' = ('10.0.0'.Length - $overlay.Length)
        'apps/lc-overlay/Cargo.toml'     = ('10.0.0'.Length - $overlay.Length)
    }
    foreach ($f in $files) {
        $after = [IO.File]::ReadAllBytes((Join-Path $repo $f))
        Assert-Equal ($before[$f].Length + $expectedDelta[$f]) $after.Length "$f length"
        $hadBom = $before[$f].Length -ge 3 -and $before[$f][0] -eq 0xEF -and $before[$f][1] -eq 0xBB -and $before[$f][2] -eq 0xBF
        $hasBom = $after.Length -ge 3 -and $after[0] -eq 0xEF -and $after[1] -eq 0xBB -and $after[2] -eq 0xBF
        Assert-Equal $hadBom $hasBom "$f BOM"
        Assert-Equal (Test-HasCrlf $before[$f]) (Test-HasCrlf $after) "$f CRLF presence"
        Assert-Equal (Test-OnlyCrlf $before[$f]) (Test-OnlyCrlf $after) "$f CRLF only"
    }
}

Test-Case 'Bump server updates the fuzz lockfile' {
    # fuzz/ is its own workspace; its lockfile repeats the version of the
    # server crates the fuzz targets link, and the nightly runs it with
    # --locked.
    $repo = New-TestRepo
    $written = @(Set-ComponentVersion -Root $repo -Component server -Version '9.0.0')
    Assert-True ($written -contains 'fuzz/Cargo.lock') "written: $($written -join ', ')"
    $lock = [IO.File]::ReadAllText((Join-Path $repo 'fuzz/Cargo.lock'))
    foreach ($package in 'lc-domain', 'lc-media', 'lc-protocol') {
        Assert-True ($lock -match ('(?m)^name = "' + $package + '"\r?\nversion = "9\.0\.0"')) "$package bumped"
    }
    Assert-Equal 1 @(Get-Versions $repo server).Count 'server consistent'
}

Test-Case 'Bump extension updates the lockfile' {
    $repo = New-TestRepo
    $lockPath = Join-Path $repo 'extension/package-lock.json'
    $lockBefore = [IO.File]::ReadAllText($lockPath)
    $current = Get-ComponentVersion -Root $repo -Component extension
    $depCount = [regex]::Matches($lockBefore, '"version": "' + [regex]::Escape($current) + '"').Count
    Set-ComponentVersion -Root $repo -Component extension -Version '50.0.0' | Out-Null
    $lockAfter = [IO.File]::ReadAllText($lockPath)
    Assert-True ($lockAfter -match '(?m)^  "version": "50\.0\.0"') 'top-level'
    Assert-True ($lockAfter -match '(?ms)^    "": \{.*?^      "version": "50\.0\.0"') 'packages[""]'
    # Windows PowerShell 5.1 cannot parse the empty `""` key of npm lockfiles.
    if ($PSVersionTable.PSVersion.Major -ge 6) {
        $parsed = $lockAfter | ConvertFrom-Json -AsHashtable
        Assert-Equal '50.0.0' $parsed['packages']['']['version'] 'parsed packages[""]'
    }
    Assert-Equal ($depCount - 2) ([regex]::Matches($lockAfter, '"version": "' + [regex]::Escape($current) + '"').Count) 'other entries untouched'
    Assert-Equal 1 @(Get-Versions $repo extension).Count 'extension consistent'
}

Test-Case 'Bump refuses lower or equal' {
    $repo = New-TestRepo
    $current = Get-ComponentVersion -Root $repo -Component server
    Assert-Throws { Set-ComponentVersion -Root $repo -Component server -Version $current } 'greater'
    Invoke-TestGit $repo tag 'server-v99.0.0' | Out-Null
    Assert-Throws { Set-ComponentVersion -Root $repo -Component server -Version '98.0.0' } 'greater'
    Assert-Throws { Set-ComponentVersion -Root $repo -Component server -Version '3.6' } 'invalid version'
    Assert-Equal $current (Get-ComponentVersion -Root $repo -Component server) 'unchanged after refusals'
}
