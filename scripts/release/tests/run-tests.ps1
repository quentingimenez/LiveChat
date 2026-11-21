# Minimal test harness for the release tooling (Pester 3 on Windows PowerShell 5.1
# is too old; this runs the same way under Windows PowerShell 5.1 and pwsh 7).
# Usage: powershell -NoProfile -File scripts/release/tests/run-tests.ps1 [-Filter <name part>]
param([string]$Filter = '')

$ErrorActionPreference = 'Stop'
$script:Failures = 0
$script:Passes = 0
$script:TempDirs = New-Object System.Collections.ArrayList
$script:TestsDir = $PSScriptRoot
$script:RepoRoot = (& git -C $PSScriptRoot rev-parse --show-toplevel).Trim()
$script:ModulePath = [IO.Path]::Combine($PSScriptRoot, '..', 'ReleaseLib.psm1')

function Test-Case {
    param([string]$Name, [scriptblock]$Body)
    if ($Filter -and $Name -notlike "*$Filter*") { return }
    try {
        & $Body
        $script:Passes++
        Write-Host "PASS $Name"
    } catch {
        $script:Failures++
        Write-Host "FAIL ${Name}: $($_.Exception.Message)"
    }
}

function Assert-Equal {
    param($Expected, $Actual, [string]$Message = '')
    if (-not ($Expected -is [string] -and $Actual -is [string]) -and ($null -eq $Expected -or $null -eq $Actual)) {
        if ($null -ne $Expected -or $null -ne $Actual) { throw "$Message expected <$Expected> got <$Actual>" }
        return
    }
    if ($Expected -ne $Actual) { throw "$Message expected <$Expected> got <$Actual>" }
}

function Assert-True {
    param([bool]$Condition, [string]$Message = 'condition is false')
    if (-not $Condition) { throw $Message }
}

function Assert-Throws {
    param([scriptblock]$Body, [string]$Like)
    $thrown = $null
    try { & $Body } catch { $thrown = $_.Exception.Message }
    if ($null -eq $thrown) { throw "expected an error like '*$Like*', nothing was thrown" }
    if ($thrown -notlike "*$Like*") { throw "expected an error like '*$Like*', got '$thrown'" }
}

function Invoke-TestGit {
    param([string]$Dir, [Parameter(ValueFromRemainingArguments = $true)][string[]]$GitArgs)
    $out = & git -C $Dir -c user.name=release-tests -c user.email=release-tests@example.invalid -c core.autocrlf=false @GitArgs 2>&1
    if ($LASTEXITCODE -ne 0) { throw "git $($GitArgs -join ' ') failed: $out" }
    return $out
}

# Copies every file the release tooling reads into a fresh git repository
# (one commit) and returns its path.
function New-TestRepo {
    $dir = Join-Path ([IO.Path]::GetTempPath()) ("lc-release-test-" + [guid]::NewGuid().ToString('N').Substring(0, 12))
    New-Item -ItemType Directory -Path $dir | Out-Null
    [void]$script:TempDirs.Add($dir)
    $files = @('Cargo.toml', 'Cargo.lock', 'fuzz/Cargo.lock', 'apps/lc-overlay/tauri.conf.json',
        'extension/package.json', 'extension/package-lock.json')
    $members = Get-Content -Raw (Join-Path $script:RepoRoot 'Cargo.toml')
    foreach ($m in [regex]::Matches(([regex]::Match($members, '(?s)members\s*=\s*\[(.*?)\]').Groups[1].Value), '"([^"]+)"')) {
        $files += ($m.Groups[1].Value + '/Cargo.toml')
    }
    foreach ($f in $files) {
        $target = Join-Path $dir $f
        $parent = Split-Path -Parent $target
        if (-not (Test-Path $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
        [IO.File]::WriteAllBytes($target, [IO.File]::ReadAllBytes((Join-Path $script:RepoRoot $f)))
    }
    Invoke-TestGit $dir init -q | Out-Null
    Invoke-TestGit $dir add -A | Out-Null
    Invoke-TestGit $dir commit -q -m initial | Out-Null
    return $dir
}

function Add-TestCommit {
    param([string]$Dir, [string]$File, [string]$Message = 'change')
    $target = Join-Path $Dir $File
    $parent = Split-Path -Parent $target
    if (-not (Test-Path $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
    [IO.File]::AppendAllText($target, "// $([guid]::NewGuid())`n")
    Invoke-TestGit $Dir add -A | Out-Null
    Invoke-TestGit $Dir commit -q -m $Message | Out-Null
}

Import-Module $script:ModulePath -Force

foreach ($testFile in Get-ChildItem -Path $PSScriptRoot -Filter '*.Tests.ps1' | Sort-Object Name) {
    . $testFile.FullName
}

foreach ($d in $script:TempDirs) { Remove-Item -Recurse -Force -LiteralPath $d -ErrorAction SilentlyContinue }
Write-Host "$script:Passes passed, $script:Failures failed"
exit $script:Failures
