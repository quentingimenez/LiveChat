# Épinglage des raccourcis iOS : protège ios/README.md contre les liens qui
# suivent une branche (main/master/HEAD) et contre les empreintes SHA-256
# périmées. Ce fichier est dot-sourcé par run-tests.ps1 : aucune action au
# chargement, seulement des définitions de fonctions et de cas de test.

function Get-IosShortcutLinks {
    $readme = Join-Path $script:RepoRoot 'ios/README.md'
    # ReadAllText avec UTF-8 explicite : le README est UTF-8 sans BOM et
    # Get-Content de Windows PowerShell 5.1 le décoderait en ANSI.
    $text = [IO.File]::ReadAllText($readme, [Text.Encoding]::UTF8)
    $pattern = 'https://raw\.githubusercontent\.com/quentingimenez/LiveChat/([^/\s)]+)/ios/(LiveChat-[^/\s)]+\.shortcut)'
    foreach ($match in [regex]::Matches($text, $pattern)) {
        [pscustomobject]@{
            Revision = $match.Groups[1].Value
            Name     = $match.Groups[2].Value
        }
    }
}

function Get-IosDocumentedHashes {
    $readme = Join-Path $script:RepoRoot 'ios/README.md'
    $text = [IO.File]::ReadAllText($readme, [Text.Encoding]::UTF8)
    $pattern = '(?m)^\|\s*`?(LiveChat-[^`|\s]+\.shortcut)`?\s*\|\s*`?([0-9a-fA-F]{64})`?\s*\|'
    $hashes = @{}
    foreach ($match in [regex]::Matches($text, $pattern)) {
        $hashes[$match.Groups[1].Value] = $match.Groups[2].Value
    }
    return $hashes
}

Test-Case 'iOS - Trois liens de raccourcis épinglés sur un même SHA de commit' {
    $links = @(Get-IosShortcutLinks)
    Assert-Equal 3 $links.Count 'ios/README.md doit contenir exactement trois liens raw.githubusercontent.com/.../ios/LiveChat-*.shortcut (ni plus ni moins)'

    $revisions = @()
    foreach ($link in $links) {
        $revision = $link.Revision
        if ($revision -match '^(main|master|HEAD)$') {
            throw "lien non épinglé sur '$revision' : la révision suit une branche. Remplacer les trois liens par un SHA de commit et mettre à jour les empreintes dans le même commit."
        }
        Assert-True ($revision -match '^[0-9a-fA-F]{40}$') "révision '$revision' n'est pas un SHA de 40 hexadécimaux : épingler les trois liens et mettre à jour les empreintes ensemble dans le même commit."
        $revisions += $revision.ToLowerInvariant()
    }
    Assert-Equal 1 @($revisions | Select-Object -Unique).Count 'les trois liens doivent partager le même SHA de commit'
}

Test-Case 'iOS - Fichiers référencés présents et empreintes documentées exactes' {
    $links = @(Get-IosShortcutLinks)
    $hashes = Get-IosDocumentedHashes

    foreach ($link in $links) {
        $file = Join-Path $script:RepoRoot (Join-Path 'ios' $link.Name)
        Assert-True (Test-Path -LiteralPath $file -PathType Leaf) "le fichier référencé $($link.Name) n'existe pas dans ios/"

        Assert-True ($hashes.ContainsKey($link.Name)) "aucune empreinte SHA-256 documentée pour $($link.Name) dans le tableau du README"
        $documented = [string]$hashes[$link.Name]
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash
        Assert-True ([string]::Equals($documented, $actual, [StringComparison]::OrdinalIgnoreCase)) "empreinte SHA-256 périmée pour $($link.Name) (documentée: $documented, réelle: $actual). Quand un .shortcut change, mettre à jour ENSEMBLE le SHA des trois liens et les trois empreintes dans le même commit : un lien et son empreinte qui divergent sont un bug."
    }
}

Test-Case 'iOS - Empreintes documentées distinctes' {
    $hashes = Get-IosDocumentedHashes
    Assert-Equal 3 $hashes.Count 'le tableau doit documenter exactement trois empreintes SHA-256'
    $values = @($hashes.Values | ForEach-Object { $_.ToLowerInvariant() })
    Assert-Equal 3 @($values | Select-Object -Unique).Count 'les trois empreintes SHA-256 doivent être distinctes (pas de copier-coller)'
}

Test-Case 'iOS - SHA épinglé égal au dernier commit des raccourcis' {
    $links = @(Get-IosShortcutLinks)
    Assert-Equal 3 $links.Count 'trois liens de raccourcis attendus'
    $pinned = $links[0].Revision.ToLowerInvariant()
    $expected = (& git -C $script:RepoRoot log -1 --format=%H -- ios/LiveChat-Photo.shortcut ios/LiveChat-Video.shortcut ios/LiveChat-Lien.shortcut).Trim().ToLowerInvariant()
    Assert-True ($expected.Length -gt 0) 'impossible de lire le dernier commit touchant les raccourcis dans git'
    Assert-Equal $expected $pinned "le SHA épinglé doit être le dernier commit touchant un .shortcut ($expected) : quand un raccourci change, mettre à jour ENSEMBLE les trois liens et les empreintes dans le même commit."
}
