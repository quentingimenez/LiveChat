# LiveChat release tool. See .claude/skills/release/SKILL.md for the full flow.
#
#   release.ps1 check                         every component has one version in all its files
#   release.ps1 plan [-Json]                  changes per component since its last release tag
#   release.ps1 bump <component> <X.Y.Z>      write a new version in every file of a component
#   release.ps1 pending-tags <baseRef> [-Json] tags to create for versions bumped since baseRef
#   release.ps1 deploy-server <X.Y.Z> [-Canary] deploy the server image on Unraid (SSH)
param(
    [Parameter(Position = 0)][string]$Command,
    [Parameter(Position = 1, ValueFromRemainingArguments = $true)][string[]]$Rest,
    [switch]$Json,
    [switch]$Canary
)

$ErrorActionPreference = 'Stop'
Import-Module ([IO.Path]::Combine($PSScriptRoot, 'release', 'ReleaseLib.psm1')) -Force
$root = (& git -C $PSScriptRoot rev-parse --show-toplevel).Trim()
try { Import-DotEnv -Root $root } catch { Write-Warning "could not read .env: $($_.Exception.Message)" }
if ($null -eq $Rest) { $Rest = @() }

# `-File` ne transmet que des arguments positionnels : une redirection PowerShell
# tapée dans cmd.exe (`*>` par exemple) arrive ici comme des arguments en trop.
# Le message de usage liste donc ce qui a été reçu, sinon le piège reste muet.
function Format-UsageError {
    param([string]$Usage, [string[]]$Arguments)
    $extra = @($Arguments | Where-Object { $null -ne $_ -and "$_".Trim().Length -gt 0 })
    if ($extra.Count -eq 0) {
        return $Usage
    }
    $shown = ($extra | ForEach-Object { "'$_'" }) -join ', '
    return "$Usage (received: $shown; under cmd.exe '*>' is not a redirection: use '> <file> 2>&1')"
}

function Invoke-Check {
    $errors = @(Test-VersionConsistency -Root $root)
    if ($errors.Count -eq 0) {
        Write-Host 'OK'
        return $true
    }
    $errors | ForEach-Object { Write-Host $_ }
    return $false
}

switch ($Command) {
    'check' {
        if (-not (Invoke-Check)) { exit 1 }
    }
    'bump' {
        if ($Rest.Count -ne 2) { throw (Format-UsageError -Usage 'usage: release.ps1 bump <server|overlay|extension> <X.Y.Z>' -Arguments $Rest) }
        $files = @(Set-ComponentVersion -Root $root -Component $Rest[0] -Version $Rest[1])
        Write-Host "$($Rest[0]) -> $($Rest[1]): $($files -join ', ')"
        if (-not (Invoke-Check)) { exit 1 }
    }
    'plan' {
        $changes = @(Get-ReleaseComponents | ForEach-Object { Get-ComponentChanges -Root $root -Component $_ })
        if ($Json) {
            ConvertTo-Json -InputObject $changes -Depth 5
            break
        }
        foreach ($c in $changes) {
            $since = $c.LastTag
            if (-not $since) { $since = '(never released)' }
            Write-Output "== $($c.Component) $($c.CurrentVersion), last release $since, $(@($c.Commits).Count) commit(s)"
            if ($c.SharedCratesChanged) {
                Write-Output '   shared crates (lc-protocol, lc-domain) changed since the last server release: release the server too'
            }
            foreach ($commit in $c.Commits) { Write-Output "   $($commit.Sha.Substring(0, 7)) $($commit.Subject)" }
        }
    }
    'pending-tags' {
        if ($Rest.Count -ne 1) { throw (Format-UsageError -Usage 'usage: release.ps1 pending-tags <baseRef> [-Json]' -Arguments $Rest) }
        $pending = @(Get-PendingReleaseTags -Root $root -BaseRef $Rest[0])
        if ($Json) {
            ConvertTo-Json -InputObject $pending -Depth 3
            break
        }
        foreach ($p in $pending) { Write-Output "$($p.Component) $($p.Tag)" }
    }
    'deploy-server' {
        if ($Rest.Count -ne 1) { throw (Format-UsageError -Usage 'usage: release.ps1 deploy-server <X.Y.Z> [-Canary]' -Arguments $Rest) }
        Invoke-ServerDeploy -Version $Rest[0] -Canary:$Canary
    }
    default {
        Write-Output 'usage: release.ps1 check | plan [-Json] | bump <component> <X.Y.Z> | pending-tags <baseRef> [-Json] | deploy-server <X.Y.Z> [-Canary]'
        exit 2
    }
}
