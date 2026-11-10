<#
.SYNOPSIS
    Build de release de l'overlay Tauri LiveChat (canal stable ou beta side-by-side)
    et collecte des metadonnees d'artefacts (temps, tailles, chemins, version info).

.DESCRIPTION
    Wrapper autour de `cargo tauri build` pour les deux configurations :
      - stable : apps/lc-overlay/tauri.conf.json        -> LiveChat      / com.livechat
      - beta   : apps/lc-overlay/tauri.beta.conf.json   -> LiveChat Next / com.livechat.next

    Le script n'installe RIEN : il ne produit que les installateurs NSIS et le binaire
    release. Les artefacts sont laisses dans target/release/bundle/nsis/ pour le Lead.

.PARAMETER Channel
    stable, beta ou both (defaut : both).

.PARAMETER SkipBuild
    Ne relance pas cargo tauri build ; collecte seulement les artefacts existants.

.PARAMETER SummaryPath
    Chemin du JSON de synthese. Defaut : target/release/bundle/release-summary-<channel>.json

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File apps/lc-overlay/scripts/release-build.ps1 -Channel both
#>
[CmdletBinding()]
param(
    [ValidateSet('stable', 'beta', 'both')]
    [string]$Channel = 'both',

    [switch]$SkipBuild,

    [string]$SummaryFile = ''
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# --- Chemins (le script vit dans apps/lc-overlay/scripts) ---------------------
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$appDir = Split-Path -Parent $scriptDir
$repoRoot = Split-Path -Parent (Split-Path -Parent $appDir)
$targetDir = Join-Path $repoRoot 'target\release'
$bundleDir = Join-Path $targetDir 'bundle\nsis'

# --- Environnement build ------------------------------------------------------
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (Test-Path $cargoBin) {
    $env:Path = "$cargoBin;$env:Path"
}
if (-not $env:CARGO_BUILD_JOBS) {
    $env:CARGO_BUILD_JOBS = '4'
}

function Format-Size([long]$bytes) {
    if ($bytes -ge 1MB) { return ('{0:N2} Mo' -f ($bytes / 1MB)) }
    if ($bytes -ge 1KB) { return ('{0:N1} Ko' -f ($bytes / 1KB)) }
    return ('{0} o' -f $bytes)
}

function Get-ChannelConfig([string]$name) {
    if ($name -eq 'stable') {
        return @{
            Name        = 'stable'
            ProductName = 'LiveChat'
            Identifier  = 'com.livechat'
            ConfigFile  = ''                       # tauri.conf.json par defaut
            Installer   = 'LiveChat_*_x64-setup.exe'
        }
    }
    return @{
        Name        = 'beta'
        ProductName = 'LiveChat Next'
        Identifier  = 'com.livechat.next'
        ConfigFile  = 'tauri.beta.conf.json'
        Installer   = 'LiveChat Next_*_x64-setup.exe'
    }
}

function Invoke-ChannelBuild([hashtable]$info) {
    $logPath = Join-Path $bundleDir ("build-{0}.log" -f $info.Name)
    $cargoArgs = @('tauri', 'build')
    if ($info.ConfigFile) {
        $cargoArgs += @('--config', $info.ConfigFile)
    }

    Write-Host ("[release-build] cargo tauri build ({0}) ..." -f $info.Name)
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $previousErrorAction = $ErrorActionPreference
    try {
        # Les diagnostics cargo arrivent sur stderr : en PS 5.1 ils sont convertis
        # en ErrorRecord, et avec ErrorActionPreference=Stop le pipeline
        # interromprait le build. On redirige tous les flux vers le log et on
        # teste $LASTEXITCODE.
        $ErrorActionPreference = 'Continue'
        Push-Location $appDir
        & cargo @cargoArgs *> $logPath
        $exitCode = $LASTEXITCODE
    }
    finally {
        Pop-Location
        $ErrorActionPreference = $previousErrorAction
    }
    $stopwatch.Stop()
    if (Test-Path $logPath) {
        Get-Content -Path $logPath -Tail 5 | ForEach-Object { Write-Host ("  | " + $_) }
    }
    return @{
        ExitCode   = $exitCode
        ElapsedSec = [math]::Round($stopwatch.Elapsed.TotalSeconds, 1)
        LogPath    = $logPath
    }
}

function Get-ArtifactInfo([hashtable]$info) {
    $installers = @()
    if (Test-Path $bundleDir) {
        $installers = @(Get-ChildItem -Path $bundleDir -Filter $info.Installer -File -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending)
    }

    $installerInfo = $null
    if ($installers.Count -gt 0) {
        $installer = $installers[0]
        $versionInfo = $null
        try { $versionInfo = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($installer.FullName) } catch { $versionInfo = $null }
        $installerInfo = @{
            Path            = $installer.FullName
            FileName        = $installer.Name
            SizeBytes       = $installer.Length
            SizeHuman       = (Format-Size $installer.Length)
            LastWriteTime   = $installer.LastWriteTime.ToString('o')
            ProductName     = $(if ($versionInfo) { $versionInfo.ProductName } else { $null })
            ProductVersion  = $(if ($versionInfo) { $versionInfo.ProductVersion } else { $null })
            FileDescription = $(if ($versionInfo) { $versionInfo.FileDescription } else { $null })
            CompanyName     = $(if ($versionInfo) { $versionInfo.CompanyName } else { $null })
        }
    }

    $exePath = Join-Path $targetDir 'lc-overlay.exe'
    $exeInfo = $null
    if (Test-Path $exePath) {
        $exe = Get-Item $exePath
        $exeVersion = $null
        try { $exeVersion = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($exe.FullName) } catch { $exeVersion = $null }
        $exeInfo = @{
            Path          = $exe.FullName
            SizeBytes     = $exe.Length
            SizeHuman     = (Format-Size $exe.Length)
            LastWriteTime = $exe.LastWriteTime.ToString('o')
            FileVersion   = $(if ($exeVersion) { $exeVersion.FileVersion } else { $null })
            ProductVersion = $(if ($exeVersion) { $exeVersion.ProductVersion } else { $null })
        }
    }

    # Copie de sûreté du binaire du canal juste après son build : le binaire
    # target/release/lc-overlay.exe est écrasé par le build du canal suivant.
    $binaryCopy = $null
    if ($exeInfo) {
        $binaryCopyPath = Join-Path $bundleDir ("lc-overlay-{0}.exe" -f $info.Name)
        Copy-Item -LiteralPath $exeInfo.Path -Destination $binaryCopyPath -Force
        $copied = Get-Item $binaryCopyPath
        $binaryCopy = @{
            Path      = $copied.FullName
            SizeBytes = $copied.Length
            SizeHuman = (Format-Size $copied.Length)
        }
    }

    return @{
        Installer       = $installerInfo
        InstallersFound = @($installers | ForEach-Object { $_.FullName })
        Exe             = $exeInfo
        BinaryCopy      = $binaryCopy
    }
}

function Write-ChannelSummary([hashtable]$info, $build, $artifacts) {
    $summary = [ordered]@{
        channel      = $info.Name
        productName  = $info.ProductName
        identifier   = $info.Identifier
        generatedAt  = (Get-Date).ToString('o')
        build        = $build
        artifacts    = $artifacts
        notes        = @(
            'Build sans signature de code (pas de certificat Authenticode) : SmartScreen peut afficher un avertissement.',
            'createUpdaterArtifacts=false : les artefacts de signature minisign (latest.json / .sig) exigent la cle privee en Phase 9 finale.',
            'Installeur NSIS currentUser : installation dans %LOCALAPPDATA%\<productName>, sans admin.'
        )
    }

    if (-not $SummaryFile) {
        $path = Join-Path $bundleDir ("release-summary-{0}.json" -f $info.Name)
    }
    elseif ($Channel -eq 'both') {
        $path = Join-Path (Split-Path -Parent $SummaryFile) ("release-summary-{0}.json" -f $info.Name)
    }
    else {
        $path = $SummaryFile
    }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -Path $path -Encoding UTF8
    return $path
}

# --- Execution ----------------------------------------------------------------
$channels = @()
if ($Channel -eq 'both' -or $Channel -eq 'stable') { $channels += Get-ChannelConfig 'stable' }
if ($Channel -eq 'both' -or $Channel -eq 'beta') { $channels += Get-ChannelConfig 'beta' }

if (-not (Test-Path $bundleDir)) {
    New-Item -ItemType Directory -Path $bundleDir -Force | Out-Null
}

$results = @()
foreach ($current in $channels) {
    $build = @{ ExitCode = 0; ElapsedSec = 0; LogPath = $null; Skipped = $true }
    if (-not $SkipBuild) {
        $build = Invoke-ChannelBuild $current
        $build.Skipped = $false
        if ($build.ExitCode -ne 0) {
            Write-Error ("cargo tauri build a echoue pour {0} (exit {1}), log : {2}" -f $current.Name, $build.ExitCode, $build.LogPath)
            continue
        }
    }

    $artifacts = Get-ArtifactInfo $current
    $summaryJsonPath = Write-ChannelSummary $current $build $artifacts

    $installer = $artifacts.Installer
    if ($null -eq $installer) {
        Write-Warning ("Aucun installateur trouve pour {0} (motif {1})" -f $current.Name, $current.Installer)
    }
    else {
        Write-Host ("[{0}] installeur : {1} ({2})" -f $current.Name, $installer.Path, $installer.SizeHuman)
    }
    Write-Host ("[{0}] synthese : {1}" -f $current.Name, $summaryJsonPath)

    $results += [pscustomobject]@{
        Channel        = $current.Name
        ProductName    = $current.ProductName
        BuildSec       = $(if ($build.Skipped) { 'skip' } else { $build.ElapsedSec })
        Installer      = $(if ($installer) { $installer.FileName } else { 'MANQUANT' })
        InstallerSize  = $(if ($installer) { $installer.SizeHuman } else { '-' })
        InstallerBytes = $(if ($installer) { $installer.SizeBytes } else { 0 })
        ExeSize        = $(if ($artifacts.Exe) { $artifacts.Exe.SizeHuman } else { '-' })
        BinaryCopy     = $(if ($artifacts.BinaryCopy) { Split-Path -Leaf $artifacts.BinaryCopy.Path } else { '-' })
        Summary        = $summaryJsonPath
    }
}

$results | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
