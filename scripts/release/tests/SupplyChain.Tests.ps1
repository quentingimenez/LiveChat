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

Test-Case 'Overlay workflow keeps signing secrets out of the build step' {
    $overlay = Get-Content -Raw (Join-Path $script:RepoRoot '.github/workflows/overlay-tauri.yml')
    $build = [regex]::Match($overlay, '(?ms)- name: Build installer \(unsigned for the updater\)\s*\r?\n(.*?)(?=\r?\n      - name:|\z)').Groups[1].Value
    Assert-True ($build.Length -gt 0) 'overlay build step not found'
    Assert-True ($build -match 'tauri\.unsigned\.conf\.json') 'overlay build must use the unsigned config'
    Assert-True ($build -notmatch 'TAURI_SIGNING_PRIVATE_KEY') 'the build step must not see the signing key'
    Assert-True ($build -notmatch 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD') 'the build step must not see the signing password'
    Assert-Equal 1 ([regex]::Matches($overlay, 'cargo tauri build --ci').Count) 'the workflow must have a single unsigned build step'

    Assert-Equal 1 ([regex]::Matches($overlay, 'secrets\.TAURI_SIGNING_PRIVATE_KEY\s*\}\}').Count) 'the key must be referenced exactly once in the workflow'
    Assert-Equal 1 ([regex]::Matches($overlay, 'secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD\s*\}\}').Count) 'the password must be referenced exactly once in the workflow'
}

Test-Case 'Overlay signing step signs the collected installer' {
    $overlay = Get-Content -Raw (Join-Path $script:RepoRoot '.github/workflows/overlay-tauri.yml')
    $sign = [regex]::Match($overlay, '(?ms)- name: Sign updater artifact\s*\r?\n(.*?)(?=\r?\n      - name:|\z)').Groups[1].Value
    Assert-True ($sign.Length -gt 0) 'overlay signing step not found'
    Assert-True ($sign -match '(?m)^        env:\s*$') 'signing step must define a step-level env block'
    Assert-True ($sign -match 'TAURI_SIGNING_PRIVATE_KEY:') 'signing step must map the private key'
    Assert-True ($sign -match 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD:') 'signing step must map the password'
    Assert-True ($sign -match 'cargo tauri signer sign') 'signing step must call tauri signer'
    Assert-True ($sign -match '--app-version') 'signing step must bind the signature to the app version'
    Assert-True ($sign -match 'dist/\*') 'signing step must sign the installer collected in dist'
    Assert-True ($sign -notmatch 'cargo tauri build') 'signing step must not run a build'
    Assert-True ($sign -notmatch '--password') 'the password must come from the environment, not argv'

    $latest = [regex]::Match($overlay, '(?ms)- name: Write latest\.json\s*\r?\n(.*?)(?=\r?\n      - name:|\z)').Groups[1].Value
    Assert-True ($latest -match 'dist/\$\(\$installer\.Name\)\.sig') 'latest.json must read the signature next to the collected installer'
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
