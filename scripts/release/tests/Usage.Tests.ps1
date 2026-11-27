# release.ps1 usage errors: a shell redirection typed where the tool expects its
# arguments must be named back to the caller.
#
# `powershell -File release.ps1 deploy-server 3.19.2 *> log 2>&1`, typed in
# cmd.exe, hands `*>`, `log` and `2>&1` to the script as positional arguments
# (cmd.exe has no PowerShell redirections). The script then refuses the call;
# the message has to show what it actually received, otherwise the operator only
# sees a generic usage line and reruns the same broken command.

function Invoke-ReleaseCli {
    param([string[]]$Arguments)
    $dir = Join-Path ([IO.Path]::GetTempPath()) ("lc-release-cli-" + [guid]::NewGuid().ToString('N').Substring(0, 12))
    New-Item -ItemType Directory -Path $dir | Out-Null
    [void]$script:TempDirs.Add($dir)
    $stdout = Join-Path $dir 'stdout.txt'
    $stderr = Join-Path $dir 'stderr.txt'
    $releaseScript = Join-Path $script:RepoRoot 'scripts/release.ps1'
    $shell = (Get-Process -Id $PID).Path
    $process = Start-Process -FilePath $shell `
        -ArgumentList (@('-NoProfile', '-File', $releaseScript) + $Arguments) `
        -WorkingDirectory $script:RepoRoot `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr `
        -Wait -PassThru -NoNewWindow
    $text = @()
    foreach ($file in @($stdout, $stderr)) {
        if (Test-Path -LiteralPath $file) { $text += (Get-Content -LiteralPath $file -Raw) }
    }
    # pwsh decorates an unhandled error with ANSI sequences and puts the wrapped
    # continuation of a long message behind a `   | ` gutter; Windows PowerShell
    # colours nothing and keeps one line. Strip the presentation so the
    # assertions state the message, not the terminal that rendered it.
    $ansi = [string][char]27 + '\[[0-9;]*[A-Za-z]'
    $plain = ($text -join "`n") -replace $ansi, ''
    $plain = $plain -replace '(?m)^[^\S\r\n]*\|[ ]?', ''
    $plain = $plain -replace '\s+', ' '
    return [pscustomobject]@{ ExitCode = $process.ExitCode; Text = $plain }
}

Test-Case 'deploy-server names the arguments it received' {
    $result = Invoke-ReleaseCli -Arguments @('deploy-server', '3.19.2', '*>', 'deploy.log', '2>&1')
    Assert-True ($result.ExitCode -ne 0) "exit code $($result.ExitCode) for a refused call"
    Assert-True ($result.Text -like '*usage: release.ps1 deploy-server*') "usage line missing: $($result.Text)"
    foreach ($received in @("'*>", "'deploy.log'", "'2>&1'")) {
        Assert-True ($result.Text -like "*$received*") "received argument $received not named: $($result.Text)"
    }
    Assert-True ($result.Text -like "*'*>' is not a redirection*") "cmd.exe hint missing: $($result.Text)"
}

Test-Case 'bump names an incomplete argument list' {
    $result = Invoke-ReleaseCli -Arguments @('bump', 'server')
    Assert-True ($result.ExitCode -ne 0) "exit code $($result.ExitCode) for a refused call"
    Assert-True ($result.Text -like '*usage: release.ps1 bump*') "usage line missing: $($result.Text)"
    Assert-True ($result.Text -like "*received: 'server';*") "received argument not named: $($result.Text)"
}
