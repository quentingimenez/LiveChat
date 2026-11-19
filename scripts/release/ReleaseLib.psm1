# Release tooling for LiveChat: version model of each released component
# (server, overlay, extension), consistency check, bump, change listing and
# Unraid deployment. Compatible with Windows PowerShell 5.1 and pwsh 7.

Set-StrictMode -Version 2.0

$script:Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$script:TagPrefixes = @{ server = 'server-v'; overlay = 'overlay-v'; extension = 'extension-v' }

function Get-ReleaseComponents {
    return @('server', 'overlay', 'extension')
}

function Assert-Component {
    param([string]$Component)
    if (-not $script:TagPrefixes.ContainsKey($Component)) {
        throw "unknown component '$Component' (expected server, overlay or extension)"
    }
}

function Get-TagPrefix {
    param([string]$Component)
    Assert-Component $Component
    return $script:TagPrefixes[$Component]
}

# ---------------------------------------------------------------- file I/O

function Read-TextFile {
    param([string]$Path)
    $bytes = [IO.File]::ReadAllBytes($Path)
    $hasBom = $bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF
    $offset = 0
    if ($hasBom) { $offset = 3 }
    return [pscustomobject]@{
        Text   = $script:Utf8NoBom.GetString($bytes, $offset, $bytes.Length - $offset)
        HasBom = $hasBom
    }
}

function Write-TextFile {
    param([string]$Path, [string]$Text, [bool]$HasBom)
    $body = $script:Utf8NoBom.GetBytes($Text)
    if ($HasBom) {
        $all = New-Object byte[] ($body.Length + 3)
        $all[0] = 0xEF; $all[1] = 0xBB; $all[2] = 0xBF
        [Array]::Copy($body, 0, $all, 3, $body.Length)
        $body = $all
    }
    [IO.File]::WriteAllBytes($Path, $body)
}

# ---------------------------------------------------------------- locators

# One location = where a version string sits in a file's text.
function New-Location {
    param([string]$Label, [System.Text.RegularExpressions.Group]$Group, [int]$Base = 0)
    return [pscustomobject]@{ Label = $Label; Index = $Base + $Group.Index; Length = $Group.Length; Value = $Group.Value }
}

# `version = "..."` of a TOML table (`[workspace.package]`, `[package]`).
function Find-TomlTableVersion {
    param([string]$Text, [string]$Table, [string]$Label)
    $header = [regex]::Match($Text, '(?m)^\[' + [regex]::Escape($Table) + '\][ \t]*\r?$')
    if (-not $header.Success) { throw "$Label : table [$Table] not found" }
    $start = $header.Index + $header.Length
    $next = [regex]::Match($Text.Substring($start), '(?m)^\[')
    $length = $Text.Length - $start
    if ($next.Success) { $length = $next.Index }
    $section = $Text.Substring($start, $length)
    $m = [regex]::Match($section, '(?m)^version[ \t]*=[ \t]*"([^"]+)"')
    if (-not $m.Success) { throw "$Label : no version in [$Table]" }
    return New-Location $Label $m.Groups[1] $start
}

# `version = "..."` right after `name = "<package>"` in Cargo.lock.
function Find-LockPackageVersion {
    param([string]$Text, [string]$Package, [string]$Label)
    $m = [regex]::Match($Text, '(?m)^name = "' + [regex]::Escape($Package) + '"\r?\nversion = "([^"]+)"')
    if (-not $m.Success) { throw "$Label : package $Package not found in Cargo.lock" }
    return New-Location $Label $m.Groups[1]
}

# Top-level `"version"` of a JSON file indented with two spaces.
function Find-JsonTopVersion {
    param([string]$Text, [string]$Label)
    $m = [regex]::Match($Text, '(?m)^  "version"[ \t]*:[ \t]*"([^"]+)"')
    if (-not $m.Success) { throw "$Label : top-level version not found" }
    return New-Location $Label $m.Groups[1]
}

# `packages[""].version` of an npm v2/v3 lockfile.
function Find-NpmLockRootVersion {
    param([string]$Text, [string]$Label)
    $root = [regex]::Match($Text, '(?m)^    "": \{')
    if (-not $root.Success) { throw "$Label : packages[`"`"] not found" }
    $start = $root.Index + $root.Length
    $m = [regex]::Match($Text.Substring($start), '(?m)^      "version"[ \t]*:[ \t]*"([^"]+)"')
    if (-not $m.Success) { throw "$Label : packages[`"`"].version not found" }
    return New-Location $Label $m.Groups[1] $start
}

# ---------------------------------------------------------------- model

# Package names of the workspace members that inherit the workspace version
# (`version.workspace = true`): they all carry the server version.
function Get-ServerWorkspaceMembers {
    param([string]$Root)
    $workspace = (Read-TextFile (Join-Path $Root 'Cargo.toml')).Text
    $list = [regex]::Match($workspace, '(?s)members\s*=\s*\[(.*?)\]')
    if (-not $list.Success) { throw 'Cargo.toml: workspace members not found' }
    $names = @()
    foreach ($m in [regex]::Matches($list.Groups[1].Value, '"([^"]+)"')) {
        $manifest = (Read-TextFile (Join-Path (Join-Path $Root $m.Groups[1].Value) 'Cargo.toml')).Text
        if ($manifest -match '(?m)^version\.workspace[ \t]*=[ \t]*true') {
            $name = [regex]::Match($manifest, '(?m)^name[ \t]*=[ \t]*"([^"]+)"')
            if (-not $name.Success) { throw "$($m.Groups[1].Value)/Cargo.toml: package name not found" }
            $names += $name.Groups[1].Value
        }
    }
    return $names
}

# Files of a component, each with a scriptblock `{ param($text, $members) }`
# giving the version locations in its text. `$members` (server workspace
# members) is only used by the server's Cargo.lock.
function Get-ComponentFiles {
    param([string]$Component)
    Assert-Component $Component
    switch ($Component) {
        'server' {
            return @(
                @{ File = 'Cargo.toml'; Locate = { param($t) Find-TomlTableVersion $t 'workspace.package' 'Cargo.toml' } },
                @{ File = 'Cargo.lock'; Locate = {
                        param($t, $members)
                        foreach ($p in $members) { Find-LockPackageVersion $t $p "Cargo.lock $p" }
                    }
                }
            )
        }
        'overlay' {
            return @(
                @{ File = 'apps/lc-overlay/tauri.conf.json'; Locate = { param($t) Find-JsonTopVersion $t 'apps/lc-overlay/tauri.conf.json' } },
                @{ File = 'apps/lc-overlay/Cargo.toml'; Locate = { param($t) Find-TomlTableVersion $t 'package' 'apps/lc-overlay/Cargo.toml' } },
                @{ File = 'Cargo.lock'; Locate = { param($t) Find-LockPackageVersion $t 'lc-overlay' 'Cargo.lock lc-overlay' } }
            )
        }
        'extension' {
            return @(
                @{ File = 'extension/public/manifest.json'; Locate = { param($t) Find-JsonTopVersion $t 'extension/public/manifest.json' } },
                @{ File = 'extension/package.json'; Locate = { param($t) Find-JsonTopVersion $t 'extension/package.json' } },
                @{ File = 'extension/package-lock.json'; Locate = {
                        param($t)
                        Find-JsonTopVersion $t 'extension/package-lock.json'
                        Find-NpmLockRootVersion $t 'extension/package-lock.json packages[""]'
                    }
                }
            )
        }
    }
}

function Get-ComponentMembers {
    param([string]$Root, [string]$Component)
    if ($Component -eq 'server') { return @(Get-ServerWorkspaceMembers -Root $Root) }
    return @()
}

# Version locations of one file of a component, from its text.
function Get-VersionLocations {
    param([string]$Component, [string]$File, [string]$Text, [string[]]$Members)
    $spec = Get-ComponentFiles -Component $Component | Where-Object { $_.File -eq $File }
    if ($null -eq $spec) { throw "$File is not a version file of $Component" }
    return @(& $spec.Locate $Text $Members)
}

function Get-ComponentVersionSources {
    param([string]$Root, [string]$Component)
    $members = Get-ComponentMembers -Root $Root -Component $Component
    $sources = @()
    foreach ($spec in Get-ComponentFiles -Component $Component) {
        $text = (Read-TextFile (Join-Path $Root $spec.File)).Text
        foreach ($loc in @(& $spec.Locate $text $members)) {
            $sources += [pscustomobject]@{ File = $spec.File; Label = $loc.Label; Version = $loc.Value }
        }
    }
    return $sources
}

# The component version = the one of its first source file.
function Get-ComponentVersion {
    param([string]$Root, [string]$Component)
    return @(Get-ComponentVersionSources -Root $Root -Component $Component)[0].Version
}

function Test-VersionConsistency {
    param([string]$Root)
    $errors = @()
    foreach ($component in Get-ReleaseComponents) {
        $sources = @(Get-ComponentVersionSources -Root $Root -Component $component)
        $reference = $sources[0]
        foreach ($s in $sources) {
            if ($s.Version -ne $reference.Version) {
                $errors += "${component}: $($s.Label) ($($s.File)) is $($s.Version), $($reference.Label) is $($reference.Version)"
            }
        }
    }
    return $errors
}

# ---------------------------------------------------------------- versions and tags

function Test-ReleaseVersion {
    param([string]$Version)
    return $Version -match '^\d+\.\d+\.\d+$'
}

function Compare-SemVer {
    param([string]$A, [string]$B)
    $pa = $A.Split('.') | ForEach-Object { [int]$_ }
    $pb = $B.Split('.') | ForEach-Object { [int]$_ }
    for ($i = 0; $i -lt 3; $i++) {
        if ($pa[$i] -gt $pb[$i]) { return 1 }
        if ($pa[$i] -lt $pb[$i]) { return -1 }
    }
    return 0
}

function Invoke-Git {
    param([string]$Root, [Parameter(ValueFromRemainingArguments = $true)][string[]]$GitArgs)
    $out = & git -C $Root @GitArgs 2>&1
    if ($LASTEXITCODE -ne 0) { throw "git $($GitArgs -join ' ') failed: $out" }
    return @($out | ForEach-Object { "$_" })
}

# Highest final (X.Y.Z) release tag of a component, or $null.
function Get-LastReleaseTag {
    param([string]$Root, [string]$Component)
    $prefix = Get-TagPrefix $Component
    $best = $null
    foreach ($tag in Invoke-Git $Root tag --list "$prefix*") {
        $tag = $tag.Trim()
        if (-not $tag.StartsWith($prefix)) { continue }
        $version = $tag.Substring($prefix.Length)
        if (-not (Test-ReleaseVersion $version)) { continue }
        if ($null -eq $best -or (Compare-SemVer $version $best.Version) -gt 0) {
            $best = [pscustomobject]@{ Tag = $tag; Version = $version }
        }
    }
    return $best
}

# Writes Version in every version location of a component and returns the
# files written. Only the version characters change (BOM and line endings kept).
function Set-ComponentVersion {
    param([string]$Root, [string]$Component, [string]$Version)
    if (-not (Test-ReleaseVersion $Version)) { throw "invalid version '$Version' (expected X.Y.Z)" }
    $floor = Get-ComponentVersion -Root $Root -Component $Component
    $last = Get-LastReleaseTag -Root $Root -Component $Component
    if ($null -ne $last -and (Compare-SemVer $last.Version $floor) -gt 0) { $floor = $last.Version }
    if ((Compare-SemVer $Version $floor) -le 0) { throw "version must be greater than $floor" }

    $members = Get-ComponentMembers -Root $Root -Component $Component
    $written = @()
    foreach ($spec in Get-ComponentFiles -Component $Component) {
        $path = Join-Path $Root $spec.File
        $file = Read-TextFile $path
        $text = $file.Text
        $locations = @(& $spec.Locate $text $members) | Sort-Object Index -Descending
        foreach ($loc in $locations) {
            $text = $text.Substring(0, $loc.Index) + $Version + $text.Substring($loc.Index + $loc.Length)
        }
        Write-TextFile -Path $path -Text $text -HasBom $file.HasBom
        $written += $spec.File
    }
    return $written
}

# ---------------------------------------------------------------- changes

$script:ChangePaths = @{
    server    = @('crates/', 'apps/lc-server/', 'apps/lc-admin-web/', 'xtask/', 'deploy/', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')
    overlay   = @('apps/lc-overlay/', 'crates/lc-protocol/', 'crates/lc-domain/')
    extension = @('extension/')
}
$script:SharedCratePaths = @('crates/lc-protocol/', 'crates/lc-domain/')

function Get-CommitsSince {
    param([string]$Root, [string]$Since, [string[]]$Paths)
    $range = 'HEAD'
    if ($Since) { $range = "$Since..HEAD" }
    $commits = @()
    foreach ($line in Invoke-Git $Root log "--format=%H%x09%s" $range '--' @Paths) {
        if (-not $line) { continue }
        $parts = $line.Split("`t", 2)
        $commits += [pscustomobject]@{ Sha = $parts[0]; Subject = $parts[1] }
    }
    return $commits
}

# Changes of a component since its last release tag (whole history when it
# has none). Overlay only: SharedCratesChanged = crates shared with the server
# changed since the last server tag (the server must then be released too).
function Get-ComponentChanges {
    param([string]$Root, [string]$Component)
    Assert-Component $Component
    $paths = $script:ChangePaths[$Component]
    $last = Get-LastReleaseTag -Root $Root -Component $Component
    $lastTag = $null
    if ($null -ne $last) { $lastTag = $last.Tag }

    $commits = @(Get-CommitsSince -Root $Root -Since $lastTag -Paths $paths)
    if ($lastTag) {
        $files = @(Invoke-Git $Root diff --name-only "$lastTag..HEAD" '--' @paths)
    } else {
        $files = @(Invoke-Git $Root log --name-only "--format=" HEAD '--' @paths)
    }
    $files = @($files | Where-Object { $_ } | Sort-Object -Unique)

    $shared = $false
    if ($Component -eq 'overlay') {
        $serverTag = Get-LastReleaseTag -Root $Root -Component server
        $since = $null
        if ($null -ne $serverTag) { $since = $serverTag.Tag }
        $shared = @(Get-CommitsSince -Root $Root -Since $since -Paths $script:SharedCratePaths).Count -gt 0
    }

    return [pscustomobject]@{
        Component           = $Component
        LastTag             = $lastTag
        CurrentVersion      = Get-ComponentVersion -Root $Root -Component $Component
        Commits             = $commits
        Files               = $files
        SharedCratesChanged = $shared
    }
}

# Version of a component at a git ref (read from its first version file).
function Get-ComponentVersionAt {
    param([string]$Root, [string]$Component, [string]$Ref)
    $spec = @(Get-ComponentFiles -Component $Component)[0]
    $text = (Invoke-Git $Root show "${Ref}:$($spec.File)") -join "`n"
    return @(& $spec.Locate $text @())[0].Value
}

# Components whose version changed between BaseRef and HEAD and whose release
# tag does not exist yet, or already points at HEAD (a re-run after a failed
# build dispatch must dispatch again; TagExists tells not to create it).
function Get-PendingReleaseTags {
    param([string]$Root, [string]$BaseRef)
    $head = "$(Invoke-Git $Root rev-parse HEAD)".Trim()
    $pending = @()
    foreach ($component in Get-ReleaseComponents) {
        $now = Get-ComponentVersionAt -Root $Root -Component $component -Ref 'HEAD'
        $before = Get-ComponentVersionAt -Root $Root -Component $component -Ref $BaseRef
        if ($now -eq $before) { continue }
        $tag = (Get-TagPrefix $component) + $now
        $exists = @(Invoke-Git $Root tag --list $tag | Where-Object { $_ }).Count -gt 0
        if ($exists -and "$(Invoke-Git $Root rev-list -n 1 $tag)".Trim() -ne $head) { continue }
        $pending += [pscustomobject]@{ Component = $component; Version = $now; Tag = $tag; TagExists = $exists }
    }
    return $pending
}

# ---------------------------------------------------------------- Unraid deploy

$script:SshExe = 'C:/Windows/System32/OpenSSH/ssh.exe'
$script:SshHost = 'root@192.0.2.59'
$script:PublicUrl = 'https://livechat.example.com'
$script:ImageName = 'ghcr.io/quentingimenez/livechat-server'

# Bash run on the Unraid host (stdin of `ssh ... bash -s`): backup, template
# update, container recreate. On any error the container is started again
# and the rollback commands are printed.
function Get-ServerDeployScript {
    param([string]$Version, [string]$Stamp, [switch]$Canary)
    if (-not (Test-ReleaseVersion $Version)) { throw "invalid version '$Version' (expected X.Y.Z)" }
    if ($Stamp -notmatch '^\d{8}-\d{6}$') { throw "invalid stamp '$Stamp' (expected yyyyMMdd-HHmmss)" }
    $lines = @(
        '#!/bin/bash',
        'set -euo pipefail',
        "VERSION='$Version'",
        "IMAGE='$($script:ImageName):$Version'",
        "TEMPLATE='/boot/config/plugins/dockerMan/templates-user/my-LiveChat.xml'",
        "DATA='/mnt/user/appdata/livechat'",
        "BACKUP='/mnt/user/appdata/livechat-backups/before-$Version-$Stamp'",
        "UPDATE='/usr/local/emhttp/plugins/dynamix.docker.manager/scripts/update_container'",
        'STOPPED=0',
        'UPDATING=0',
        'PREV_IMAGE="$(sed -n ''s:.*<Repository>\(.*\)</Repository>.*:\1:p'' "$TEMPLATE" | head -n 1)"',
        'on_error() {',
        '  echo "DEPLOY FAILED (line $1)" >&2',
        '  if [ "$UPDATING" = 1 ] && [ -f "$BACKUP/my-LiveChat.xml" ]; then',
        '    cp "$BACKUP/my-LiveChat.xml" "$TEMPLATE"',
        '    "$UPDATE" LiveChat >/dev/null 2>&1 || true',
        '  fi',
        '  if [ "$STOPPED" = 1 ]; then docker start LiveChat >/dev/null 2>&1 || true; fi',
        '  echo "previous image: $PREV_IMAGE; container: $(docker ps -a --filter name=^LiveChat$ --format ''{{.Status}}'' 2>/dev/null)" >&2',
        '  echo "ROLLBACK (if needed): cp $BACKUP/my-LiveChat.xml $TEMPLATE; $UPDATE LiveChat; docker start LiveChat  (database backup in $BACKUP)" >&2',
        '}',
        'trap ''on_error $LINENO'' ERR',
        '[ -n "$PREV_IMAGE" ]',
        'echo "previous image: $PREV_IMAGE"',
        'docker pull "$IMAGE"',
        'docker stop -t 20 LiveChat',
        'STOPPED=1',
        'mkdir -p "$BACKUP"',
        'cp -a "$DATA"/livechat.sqlite* "$TEMPLATE" "$BACKUP"/',
        'for f in "$DATA"/livechat.sqlite* "$TEMPLATE"; do cmp "$f" "$BACKUP/$(basename "$f")"; done',
        'sed -i "s|<Repository>.*</Repository>|<Repository>$IMAGE</Repository>|" "$TEMPLATE"',
        'grep -q "<Repository>$IMAGE</Repository>" "$TEMPLATE"',
        'UPDATING=1',
        '"$UPDATE" LiveChat',
        'docker start LiveChat',
        'STOPPED=0',
        'UPDATING=0',
        'echo "DEPLOYED $VERSION BACKUP $BACKUP PREVIOUS $PREV_IMAGE"',
        # PowerShell appends a CRLF after piped text: exit before bash reads it.
        'exit 0'
    )
    if ($Canary) {
        # Canary gate: boot the new image as LiveChat-canary on a copy of the
        # backup, on the test bot env file, health-check it on 127.0.0.1:3001,
        # and only then let the deploy touch the template. The env file is
        # created once by the user on the host, never by this script
        # (.claude/skills/release/SKILL.md).
        $canaryVars = [string[]]@(
            "CANARY_DIR='/mnt/user/appdata/livechat-canary'",
            "CANARY_ENV='/mnt/user/appdata/livechat-canary.env'",
            'CANARY=0'
        )
        $envGate = [string[]]@(
            'if [ -f "$CANARY_ENV" ]; then',
            '  :',
            'else',
            '  echo "canary env file $CANARY_ENV missing - see the release skill" >&2',
            '  exit 1',
            'fi'
        )
        $canaryCleanup = [string[]]@(
            '  if [ "$CANARY" = 1 ]; then',
            '    echo "CANARY FAILED" >&2',
            '    docker rm -f LiveChat-canary >/dev/null 2>&1 || true',
            '  fi'
        )
        $canaryGate = [string[]]@(
            'docker rm -f LiveChat-canary >/dev/null 2>&1 || true',
            'rm -rf "$CANARY_DIR"',
            'mkdir -p "$CANARY_DIR/media"',
            'cp -a "$BACKUP"/livechat.sqlite* "$CANARY_DIR"/',
            'chown -R 10001:10001 "$CANARY_DIR"',
            'CANARY=1',
            'docker run -d --name LiveChat-canary -p 127.0.0.1:3001:3000 -v "$CANARY_DIR":/app/data --env-file "$CANARY_ENV" -e API_URL=http://127.0.0.1:3001 "$IMAGE"',
            'body=""',
            'canary_ok=0',
            'for i in $(seq 1 45); do',
            '  body="$(curl -fsS --max-time 5 http://127.0.0.1:3001/health 2>/dev/null || true)"',
            '  if echo "$body" | grep -q "\"version\":\"$VERSION\"" && echo "$body" | grep -q ''"db":"ok"'' && echo "$body" | grep -q ''"discord":"ok"''; then',
            '    canary_ok=1',
            '    break',
            '  fi',
            '  sleep 2',
            'done',
            'if [ "$canary_ok" != 1 ]; then',
            '  echo "canary health check failed" >&2',
            '  false',
            'fi',
            'if [ "$(docker logs LiveChat-canary 2>&1 | grep -c ''Discord pr'')" -lt 1 ]; then',
            '  echo "canary did not log in to discord" >&2',
            '  false',
            'fi',
            'docker rm -f LiveChat-canary >/dev/null 2>&1 || true',
            'CANARY=0',
            'echo "CANARY OK $VERSION"'
        )
        $varsAdded = $false
        $withCanary = New-Object System.Collections.Generic.List[string]
        foreach ($line in $lines) {
            if ($line -ceq 'sed -i "s|<Repository>.*</Repository>|<Repository>$IMAGE</Repository>|" "$TEMPLATE"') { $withCanary.AddRange($canaryGate) }
            $withCanary.Add($line)
            if (-not $varsAdded -and $line -ceq 'UPDATING=0') {
                $varsAdded = $true
                $withCanary.AddRange($canaryVars)
            } elseif ($line -ceq '[ -n "$PREV_IMAGE" ]') {
                $withCanary.AddRange($envGate)
            } elseif ($line -ceq '  echo "DEPLOY FAILED (line $1)" >&2') {
                $withCanary.AddRange($canaryCleanup)
            }
        }
        $lines = $withCanary.ToArray()
    }
    return ($lines -join "`n") + "`n"
}

# Checks run on the host once /health reports the new version: the Discord
# bot logged in, and /admin/ is refused for a non-allowed source address.
# ASCII only: `Discord pr` matches the "Discord pret" line without
# depending on the host locale.
function Get-ServerCheckScript {
    return (@(
            'set -u',
            'echo "discord_ready=$(docker logs --since 15m LiveChat 2>&1 | grep -c ''Discord pr'' || true)"',
            'echo "admin_status=$(curl -sk -o /dev/null -w ''%{http_code}'' --resolve livechat.example.com:4433:127.0.0.1 https://livechat.example.com:4433/admin/ || true)"',
            'exit 0'
        ) -join "`n") + "`n"
}

# Commands that put the previous image and database back, printed when a
# deploy fails after the host script succeeded (new image unhealthy).
function Get-ServerRollbackCommand {
    param([string]$Backup)
    $template = '/boot/config/plugins/dockerMan/templates-user/my-LiveChat.xml'
    $update = '/usr/local/emhttp/plugins/dynamix.docker.manager/scripts/update_container'
    return "docker stop -t 20 LiveChat; cp $Backup/my-LiveChat.xml $template; " +
        "cp -a $Backup/livechat.sqlite* /mnt/user/appdata/livechat/; $update LiveChat; docker start LiveChat"
}

# Backup dir and previous image from the host script's `DEPLOYED ...` line.
function Get-DeployResult {
    param([string[]]$Lines)
    foreach ($line in $Lines) {
        $m = [regex]::Match($line, '^\s*DEPLOYED (\S+) BACKUP (\S+) PREVIOUS (\S+)\s*$')
        if ($m.Success) { return [pscustomobject]@{ Version = $m.Groups[1].Value; Backup = $m.Groups[2].Value; Previous = $m.Groups[3].Value } }
    }
    throw 'no DEPLOYED line in the host output'
}

function Invoke-RemoteBash {
    param([string]$Script)
    # Windows PowerShell 5.1 writes piped text with the console input encoding:
    # when it is UTF-8 it carries a BOM, which breaks the script's first line.
    $savedInput = [Console]::InputEncoding
    $savedOutput = $OutputEncoding
    try {
        [Console]::InputEncoding = $script:Utf8NoBom
        $OutputEncoding = $script:Utf8NoBom
        $out = $Script | & $script:SshExe -o BatchMode=yes $script:SshHost 'bash -s' 2>&1
        $code = $LASTEXITCODE
    } finally {
        [Console]::InputEncoding = $savedInput
        $OutputEncoding = $savedOutput
    }
    $text = @($out | ForEach-Object { "$_" })
    $text | ForEach-Object { Write-Host "  [host] $_" }
    if ($code -ne 0) { throw "remote script failed (exit $code)" }
    return $text
}

function Wait-ServerHealth {
    param([string]$Version, [int]$TimeoutSec = 120)
    # The public proxy requires TLS 1.3, which Windows PowerShell 5.1 leaves off.
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]'Tls12, Tls13'
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $last = 'no answer'
    while ((Get-Date) -lt $deadline) {
        try {
            $h = Invoke-RestMethod -Uri "$($script:PublicUrl)/health" -TimeoutSec 10
            $last = "version=$($h.version) db=$($h.db) discord=$($h.discord)"
            if ($h.version -eq $Version -and $h.db -eq 'ok' -and $h.discord -eq 'ok') { return $h }
        } catch {
            $last = $_.Exception.Message
        }
        Start-Sleep -Seconds 5
    }
    throw "/health did not report $Version with db and discord ok within $TimeoutSec s (last: $last)"
}

# Deploys server image Version on the Unraid host, then checks /health, the
# Discord login and the /admin/ restriction. Throws on the first failure.
function Invoke-ServerDeploy {
    param([string]$Version, [switch]$Canary)
    $stamp = (Get-Date).ToString('yyyyMMdd-HHmmss')
    $script = Get-ServerDeployScript -Version $Version -Stamp $stamp -Canary:$Canary
    Write-Host "Deploying $($script:ImageName):$Version on $($script:SshHost)"
    $result = Get-DeployResult -Lines (Invoke-RemoteBash $script)
    try {
        $h = Wait-ServerHealth -Version $Version
        Write-Host "health: version=$($h.version) protocol=$($h.protocolVersion) db=$($h.db) discord=$($h.discord)"
        $checks = Invoke-RemoteBash (Get-ServerCheckScript)
        $ready = [int](($checks | Where-Object { $_ -like 'discord_ready=*' }) -replace '^discord_ready=', '')
        $admin = (($checks | Where-Object { $_ -like 'admin_status=*' }) -replace '^admin_status=', '')
        if ($ready -lt 1) { throw "no 'Discord pret' line in the container logs" }
        if ($admin -ne '403') { throw "/admin/ from a non-allowed address returned $admin instead of 403" }
    } catch {
        Write-Host "DEPLOY CHECK FAILED: $($_.Exception.Message)"
        Write-Host "Previous image: $($result.Previous); backup: $($result.Backup)"
        Write-Host "ROLLBACK (on the host): $(Get-ServerRollbackCommand -Backup $result.Backup)"
        throw
    }
    Write-Host "Server $Version deployed and checked (backup $($result.Backup))."
}

Export-ModuleMember -Function Get-ReleaseComponents, Get-ComponentVersionSources, Get-ComponentVersion,
    Get-ServerWorkspaceMembers, Get-VersionLocations, Test-VersionConsistency, Test-ReleaseVersion,
    Compare-SemVer, Get-LastReleaseTag, Get-TagPrefix, Set-ComponentVersion,
    Get-ComponentChanges, Get-PendingReleaseTags, Get-ServerDeployScript, Get-ServerCheckScript, Get-ServerRollbackCommand, Get-DeployResult, Invoke-ServerDeploy
