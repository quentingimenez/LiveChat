# deploy-server -Canary: the bash script sent to the Unraid host with a canary
# container gate (test bot, copy of the backup, health check on 127.0.0.1:3001).

# Same helper as Deploy.Tests.ps1: files are dot-sourced in name order
# (Bump, Canary, Deploy, ...), so this one cannot rely on the later definition.
function Get-StepIndex {
    param([string]$Script, [string]$Needle)
    $i = $Script.IndexOf($Needle)
    if ($i -lt 0) { throw "step '$Needle' missing" }
    return $i
}

Test-Case 'Canary script order' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000' -Canary
    $order = '[ -f "$CANARY_ENV" ]', 'docker stop -t 20 LiveChat', 'cmp ',
        'docker run -d --name LiveChat-canary', 'docker rm -f LiveChat-canary', 'CANARY OK',
        'sed -i "s|<Repository>', '"$UPDATE" LiveChat'
    $previous = -1
    foreach ($step in $order) {
        # `docker rm -f LiveChat-canary` also appears in the error trap and at the
        # top of the gate: the one required here is the success-path removal,
        # between `docker run` and `CANARY OK`.
        $i = $s.IndexOf($step, $previous + 1)
        Assert-True ($i -gt $previous) "step '$step' is not after the previous one"
        $previous = $i
    }
}

Test-Case 'Canary uses its own data dir and the test bot env file' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000' -Canary
    $run = @($s -split "`n" | Where-Object { $_.StartsWith('docker run -d --name LiveChat-canary') })
    Assert-True ($run.Count -eq 1) 'one canary docker run line'
    Assert-True ($run[0].Contains('-v "$CANARY_DIR":/app/data')) 'canary data dir mounted'
    Assert-True ($run[0].Contains('--env-file "$CANARY_ENV"')) 'test bot env file'
    Assert-True ($run[0].Contains('-p 127.0.0.1:3001:3000')) 'loopback-only canary port'
    Assert-True ($run[0] -cnotlike '*$DATA*') 'canary does not mount the main data dir'
    Assert-True (-not $s.Contains('DISCORD_TOKEN')) 'no token in the script'
    Assert-True (-not $s.Contains('TIKTOK_COOKIE')) 'no cookie in the script'
    Assert-True (-not $s.Contains('docker inspect')) 'no docker inspect'
    Assert-True (-not $s.Contains('Config.Env')) 'no env dump'
}

Test-Case 'Canary never stops or recreates the main container' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000' -Canary
    $start = Get-StepIndex $s 'CANARY=1'
    $end = $s.IndexOf('CANARY=0', $start + 1)
    Assert-True ($end -gt $start) 'CANARY=0 closes the canary window'
    $window = $s.Substring($start, $end - $start)
    Assert-True ($window.Contains('docker run -d --name LiveChat-canary')) 'canary runs inside the window'
    Assert-True ($window -cnotlike '*docker stop*') 'no docker stop in the canary window'
    Assert-True ($window -cnotlike '*"$UPDATE"*') 'no update_container in the canary window'
    foreach ($line in @($window -split "`n" | Where-Object { $_ -cmatch 'docker' })) {
        Assert-True ($line -cmatch 'LiveChat-canary') "docker command without the canary name: $line"
    }
}

Test-Case 'Canary failure removes the canary and restarts the main container' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000' -Canary
    $trap = [regex]::Match($s, "(?s)on_error\(\) \{(.*?)\n\}")
    Assert-True $trap.Success 'on_error function'
    $body = $trap.Groups[1].Value
    Assert-True ($body.Contains('CANARY FAILED')) 'trap announces the canary failure'
    Assert-True ($body.Contains('docker rm -f LiveChat-canary')) 'trap removes the canary'
    Assert-True ($body.Contains('docker start LiveChat')) 'trap restarts the main container'
    Assert-True ($body.Contains('UPDATING')) 'trap knows whether update_container ran'
    Assert-True ($body.IndexOf('CANARY FAILED') -lt $body.IndexOf('UPDATING')) 'canary cleanup runs first'
}

Test-Case 'Canary failures go through the error trap' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000' -Canary
    $start = Get-StepIndex $s 'CANARY=1'
    $end = $s.IndexOf('CANARY=0', $start + 1)
    Assert-True ($end -gt $start) 'CANARY=0 closes the canary window'
    $window = $s.Substring($start, $end - $start)
    Assert-True (-not $window.Contains('exit 1')) 'canary failures must use false so the ERR trap fires'
}

Test-Case 'Deploy script without canary is unchanged' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000'
    Assert-True ($s -notmatch 'canary') 'no canary in the default script'
    Assert-True ($s -notmatch 'CANARY') 'no CANARY flag in the default script'
    Assert-True ($s -notmatch '3001') 'no canary port in the default script'
}

Test-Case 'Canary script is ASCII only and has no CR' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000' -Canary
    Assert-True (-not $s.Contains("`r")) 'no CR'
    Assert-True ($s -notmatch '[^\x00-\x7F]') 'non-ASCII character in the canary script'
}

Test-Case 'release.ps1 parses cleanly' {
    $release = Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) 'release.ps1'
    $tokens = $null; $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($release, [ref]$tokens, [ref]$errors)
    Assert-True (@($errors).Count -eq 0) "release.ps1 parse errors: $(@($errors).Count)"
}

Test-Case 'Deploy refuses invalid version with canary too' {
    Assert-Throws { Get-ServerDeployScript -Version 'latest' -Stamp '20261117-120000' -Canary } 'invalid version'
}
