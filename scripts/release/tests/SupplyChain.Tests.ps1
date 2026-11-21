# Supply-chain invariants for the extension release workflow.
# Protected here: the AMO credentials are scoped to the signing step only, and
# web-ext is installed from a committed lockfile at an exact version with
# integrity, instead of being fetched by `npx --yes`.

$workflowPath = Join-Path $script:RepoRoot '.github/workflows/extension.yml'
$workflow = Get-Content -Raw $workflowPath

Test-Case 'AMO workflow keeps credentials out of the job environment' {
    $amo = [regex]::Match($workflow, '(?ms)^  publish-amo:\s*\r?\n(.*)\z').Groups[1].Value
    Assert-True ($amo.Length -gt 0) 'publish-amo job not found'

    # A job-level env block is indented four spaces. The secrets must only be
    # attached to the signing step, after checkout, npm ci and unpack.
    Assert-True ($amo -notmatch '(?m)^    env:\s*$') 'publish-amo must not define a job-level env block'
    Assert-True ($amo -notmatch '(?m)^      AMO_JWT_ISSUER:') 'AMO secrets must not be exposed at job scope'
    Assert-True ($amo -notmatch '(?m)^      AMO_JWT_SECRET:') 'AMO secrets must not be exposed at job scope'
    Assert-True ($amo -match 'persist-credentials:\s*false') 'publish-amo checkout must not persist credentials'
    Assert-Equal 1 ([regex]::Matches($amo, 'secrets\.AMO_JWT_ISSUER').Count) 'the AMO issuer secret must be referenced exactly once in the job'
    Assert-Equal 1 ([regex]::Matches($amo, 'secrets\.AMO_JWT_SECRET').Count) 'the AMO JWT secret must be referenced exactly once in the job'
}

Test-Case 'AMO signing step receives the credentials and uses the pinned binary' {
    $step = [regex]::Match($workflow, '(?ms)- name: Submit the new version to AMO\s*\r?\n(.*?)(?=\r?\n      - name:|\z)').Groups[1].Value
    Assert-True ($step.Length -gt 0) 'AMO signing step not found'

    Assert-True ($step -match '(?m)^        env:\s*$') 'signing step must define a step-level env block'
    Assert-True ($step -match '(?m)^          WEB_EXT_API_KEY:\s*\$\{\{\s*secrets\.AMO_JWT_ISSUER\s*\}\}\s*$') 'WEB_EXT_API_KEY must map to the AMO issuer secret'
    Assert-True ($step -match '(?m)^          WEB_EXT_API_SECRET:\s*\$\{\{\s*secrets\.AMO_JWT_SECRET\s*\}\}\s*$') 'WEB_EXT_API_SECRET must map to the AMO secret'
    Assert-True ($step -notmatch '--api-key|--api-secret') 'credentials must not be passed in argv'

    Assert-True ($workflow -notmatch 'npx --yes') 'workflow must not fetch web-ext through npx --yes'
    Assert-True ($workflow -match 'npm ci --ignore-scripts') 'signing tool must be installed with npm ci --ignore-scripts'
    Assert-True ($workflow -match '\.\./tools/amo-sign/node_modules/\.bin/web-ext sign') 'workflow must call the local pinned web-ext binary'
}

Test-Case 'AMO signing tool is pinned with a lockfile integrity' {
    $package = Get-Content -Raw (Join-Path $script:RepoRoot 'tools/amo-sign/package.json') | ConvertFrom-Json
    $pin = $package.devDependencies.PSObject.Properties['web-ext'].Value
    Assert-Equal '8.10.0' $pin 'web-ext must be pinned exactly'

    Push-Location $script:RepoRoot
    try {
        $lockJson = & node -e "const l = require('./tools/amo-sign/package-lock.json'); const p = l.packages['node_modules/web-ext']; console.log(JSON.stringify({version: p.version, integrity: p.integrity, lockfileVersion: l.lockfileVersion}));" 2>&1
        if ($LASTEXITCODE -ne 0) { throw "node failed to read the lockfile: $lockJson" }
    } finally {
        Pop-Location
    }

    $lock = $lockJson | ConvertFrom-Json
    Assert-Equal '8.10.0' $lock.version 'lockfile web-ext version must match the pin'
    Assert-Equal 3 $lock.lockfileVersion 'lockfile must use lockfileVersion 3'
    Assert-True ($lock.integrity -match '^sha512-') 'web-ext must carry a sha512 integrity'
}
