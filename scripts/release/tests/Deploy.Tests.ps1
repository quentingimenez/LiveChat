# deploy-server: the bash script sent to the Unraid host.

function Get-StepIndex {
    param([string]$Script, [string]$Needle)
    $i = $Script.IndexOf($Needle)
    if ($i -lt 0) { throw "step '$Needle' missing" }
    return $i
}

Test-Case 'Deploy script order' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000'
    Assert-True ($s.Contains("IMAGE='ghcr.io/quentingimenez/livechat-server:3.6.0'")) 'image'
    $order = 'docker pull "$IMAGE"', 'docker stop -t 20 LiveChat', 'cmp ',
        '<Repository>', '"$UPDATE" LiveChat', 'docker start LiveChat', 'DEPLOYED $VERSION'
    $previous = -1
    foreach ($step in $order) {
        # `docker start LiveChat` also appears in the error trap: look after the previous step.
        $i = $s.IndexOf($step, $previous + 1)
        Assert-True ($i -gt $previous) "step '$step' is not after the previous one"
        $previous = $i
    }
    Assert-True ($s.Contains('/mnt/user/appdata/livechat-backups/before-3.6.0-20261117-120000')) 'backup dir'
    Assert-True ($s.StartsWith("#!/bin/bash`nset -euo pipefail")) 'strict bash'
}

Test-Case 'Deploy script restarts container on error' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000'
    $trap = [regex]::Match($s, "(?s)on_error\(\) \{(.*?)\n\}")
    Assert-True $trap.Success 'on_error function'
    Assert-True ($trap.Groups[1].Value.Contains('docker start LiveChat')) 'trap restarts the container'
    Assert-True ($trap.Groups[1].Value.Contains('PREV_IMAGE')) 'trap prints the rollback with the previous image'
    Assert-True ($s -match "(?m)^trap 'on_error[^']*' ERR$") 'trap installed'
    Assert-True ((Get-StepIndex $s 'PREV_IMAGE=') -lt (Get-StepIndex $s 'docker stop -t 20 LiveChat')) 'previous image read before stopping'
}

Test-Case 'Deploy script has no CR and no secrets' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000'
    Assert-True (-not $s.Contains("`r")) 'no CR'
    Assert-True (-not $s.Contains('docker inspect')) 'no docker inspect'
    Assert-True (-not $s.Contains('Config.Env')) 'no env dump'
}

Test-Case 'Deploy refuses invalid version' {
    Assert-Throws { Get-ServerDeployScript -Version 'latest' -Stamp '20261117-120000' } 'invalid version'
}

Test-Case 'Remote scripts are ASCII only' {
    # Non-ASCII bytes are mangled by the PowerShell -> ssh pipe and by the
    # host's C locale (`.` matches one byte, an e-circumflex is two bytes in UTF-8).
    foreach ($s in (Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000'), (Get-ServerCheckScript)) {
        Assert-True ($s -notmatch '[^\x00-\x7F]') 'non-ASCII character in a remote script'
    }
    Assert-True ((Get-ServerCheckScript).Contains("grep -c 'Discord pr'")) 'Discord login check'
}

Test-Case 'Deploy error after update restores the template and recreates' {
    $s = Get-ServerDeployScript -Version '3.6.0' -Stamp '20261117-120000'
    $trap = [regex]::Match($s, "(?s)on_error\(\) \{(.*?)\n\}").Groups[1].Value
    Assert-True ($trap.Contains('UPDATING')) 'trap knows whether update_container ran'
    Assert-True ($trap.Contains('cp "$BACKUP/my-LiveChat.xml" "$TEMPLATE"')) 'trap restores the template'
    Assert-True ($trap.Contains('"$UPDATE" LiveChat')) 'trap recreates the container'
    Assert-True ((Get-StepIndex $s 'UPDATING=1') -lt $s.IndexOf('"$UPDATE" LiveChat', (Get-StepIndex $s 'UPDATING=1'))) 'flag set before update_container'
}

Test-Case 'Rollback command restores template and database' {
    $r = Get-ServerRollbackCommand -Backup '/mnt/user/appdata/livechat-backups/before-3.6.0-20261117-120000'
    Assert-True ($r.Contains('cp /mnt/user/appdata/livechat-backups/before-3.6.0-20261117-120000/my-LiveChat.xml /boot/config/plugins/dockerMan/templates-user/my-LiveChat.xml')) "template restore: $r"
    Assert-True ($r.Contains('docker stop -t 20 LiveChat')) 'stop before restoring the database'
    Assert-True ($r.Contains('livechat.sqlite')) 'database restore'
    Assert-True ($r.Contains('update_container LiveChat')) 'recreate'
    Assert-True ($r.Contains('docker start LiveChat')) 'start'
}

Test-Case 'Deploy result is parsed from the host output' {
    $lines = @('  previous image: x', 'DEPLOYED 3.6.0 BACKUP /mnt/user/appdata/livechat-backups/before-3.6.0-20261117-120000 PREVIOUS ghcr.io/quentingimenez/livechat-server:3.5.1')
    $r = Get-DeployResult -Lines $lines
    Assert-Equal '/mnt/user/appdata/livechat-backups/before-3.6.0-20261117-120000' $r.Backup 'backup'
    Assert-Equal 'ghcr.io/quentingimenez/livechat-server:3.5.1' $r.Previous 'previous'
    Assert-Throws { Get-DeployResult -Lines @('nothing') } 'DEPLOYED'
}
