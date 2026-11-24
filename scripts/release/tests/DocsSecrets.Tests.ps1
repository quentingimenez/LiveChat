# Garde-fou des exemples de jeton : les blocs de commandes de
# docs/ios-share-shortcut.md doivent saisir le jeton sans écho et sans le
# recopier en clair, et le placeholder piège ne doit plus apparaître dans les
# documents Markdown de docs/, ios/ et extension/. Le jeton d'ingestion est un
# secret d'écriture : un exemple qui le recopie l'expose dans
# ConsoleHost_history.txt / ~/.bash_history, dans la ligne de commande du
# processus et dans les messages. Ce fichier est dot-sourcé par run-tests.ps1 :
# aucune action au chargement, seulement des définitions.

function Get-IosDocText {
    $path = Join-Path $script:RepoRoot 'docs/ios-share-shortcut.md'
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "documentation manquante : $path"
    }
    # ReadAllText avec UTF-8 explicite : Get-Content de Windows PowerShell 5.1
    # décoderait le texte accentué en ANSI.
    return [IO.File]::ReadAllText($path, [Text.Encoding]::UTF8)
}

function Get-IosDocFencedCodeBlocks {
    param([string]$Language)
    $text = Get-IosDocText
    $pattern = '(?ms)^```' + [regex]::Escape($Language) + '\s*\r?\n(.*?)^```\s*$'
    $blocks = @()
    foreach ($match in [regex]::Matches($text, $pattern)) {
        $blocks += $match.Groups[1].Value
    }
    return $blocks
}

function Get-IosDocCommandBlocks {
    $blocks = @()
    $blocks += @(Get-IosDocFencedCodeBlocks -Language 'powershell')
    $blocks += @(Get-IosDocFencedCodeBlocks -Language 'bash')
    $blocks += @(Get-IosDocFencedCodeBlocks -Language 'sh')
    $blocks += @(Get-IosDocFencedCodeBlocks -Language 'shell')
    return $blocks
}

Test-Case 'Docs secrets - aucun jeton littéral assigné dans le bloc PowerShell' {
    $blocks = @(Get-IosDocFencedCodeBlocks -Language 'powershell')
    Assert-True ($blocks.Count -gt 0) 'aucun bloc ```powershell trouvé dans docs/ios-share-shortcut.md'
    $literal = '(?m)(?i)^\s*\$token\s*=\s*["'']'
    foreach ($block in $blocks) {
        Assert-True ($block -notmatch $literal) `
            "affectation `$token = '<littéral>' trouvée : le jeton doit être saisi avec Read-Host -AsSecureString puis converti via [Net.NetworkCredential]::new('', `$secure).Password, jamais recopié dans la ligne de commande."
    }
    # La regex « pas de littéral » seule passerait si la conversion était
    # supprimée ou remplacée par $secure.ToString() : exiger la conversion
    # réelle dans le bloc PowerShell documenté, pas seulement une chaîne en dur.
    $ps = [string]::Join("`n", $blocks)
    # \x27 = apostrophe : évite de doubler les guillemets dans le motif PS.
    $conversion = '\[Net\.NetworkCredential\]::new\(\x27\x27,\s*\$secure\)\.Password'
    Assert-True ($ps -match $conversion) `
        'le bloc PowerShell doit convertir la SecureString via [Net.NetworkCredential]::new('''', $secure).Password (compatible PS 5.1)'
    # L'invariant doit mordre : la conversion autorisée ne doit pas être
    # confondue avec un littéral, et l'ancien example doit être détecté.
    $old = '$token = "abcdefghij1234567890"'
    $new = '$token = [Net.NetworkCredential]::new('''', $secure).Password'
    Assert-True ($old -match $literal) 'la regex doit refuser l''ancien exemple avec jeton littéral'
    Assert-True ($new -notmatch $literal) 'la regex doit accepter la conversion SecureString autorisée'
}

Test-Case 'Docs secrets - saisie sans echo PowerShell et bash' {
    $psBlocks = @(Get-IosDocFencedCodeBlocks -Language 'powershell')
    $ps = [string]::Join("`n", $psBlocks)
    Assert-True ($ps -match 'Read-Host\s+[^\r\n]*-AsSecureString') `
        'le bloc PowerShell doit saisir le jeton avec Read-Host -AsSecureString (sans écho, hors historique)'

    $bashBlocks = @(Get-IosDocFencedCodeBlocks -Language 'bash')
    Assert-True ($bashBlocks.Count -gt 0) 'aucun bloc ```bash trouvé dans docs/ios-share-shortcut.md'
    $bash = [string]::Join("`n", $bashBlocks)
    Assert-True ($bash -match '(?m)(?i)(^|[;&|]\s*)read\s+-[a-z]*s[a-z]*\s+[^\r\n]*TOKEN') `
        'le bloc bash doit lire le jeton avec read -s (sans écho)'
    Assert-True ($bash -match '(?m)(?i)^\s*unset\s+TOKEN\s*$') `
        'le bloc bash doit effacer TOKEN avec unset après la requête'
}

Test-Case 'Docs secrets - aucun jeton en clair dans la documentation iOS' {
    $blocks = @(Get-IosDocCommandBlocks)
    Assert-True ($blocks.Count -gt 0) 'aucun bloc de commandes trouvé dans docs/ios-share-shortcut.md'
    $commands = [string]::Join("`n", $blocks)
    # Le document entier est scanné : un jeton collé en prose est tout aussi
    # dangereux qu'un jeton dans un bloc. Vérifié sans faux positif sur la doc
    # actuelle (les seuls `<Jeton>` / `<ADMIN_UI_TOKEN>` y sont des placeholders).
    $source = Get-IosDocText
    $originalStart = $source.IndexOf('$token = "…jeton affiché par la board…"')
    if ($originalStart -ge 0) {
        throw 'l''ancien exemple avec jeton en clair ($token = "…jeton affiché par la board…") est de retour dans la documentation'
    }
    $bearerLiteral = '(?i)Bearer\s+[A-Za-z0-9_\-\.]{20,}'
    $hexLiteral = '(?i)\b[0-9a-f]{32,}\b'
    # Blocs : message d'erreur ciblé ; document entier : même invariant en prose.
    Assert-True ($commands -notmatch $bearerLiteral) `
        'un jeton littéral de 20+ caractères apparaît après Bearer dans un bloc de commandes : utiliser la variable lue sans écho.'
    Assert-True ($source -notmatch $bearerLiteral) `
        'un jeton littéral de 20+ caractères apparaît après Bearer dans la documentation (même en prose) : utiliser un placeholder explicite.'
    Assert-True ($commands -notmatch $hexLiteral) `
        'une chaîne hexadécimale de 32+ caractères apparaît dans un bloc de commandes : ce doit être un secret saisi, pas un littéral de doc.'
    Assert-True ($source -notmatch $hexLiteral) `
        'une chaîne hexadécimale de 32+ caractères apparaît dans la documentation (même en prose) : ce doit être un secret saisi, pas un littéral de doc.'
    Assert-True ('Bearer A1B2C3D4E5F6A7B8C9D0E1F2A3B4C5D6' -match $bearerLiteral) `
        'la regex Bearer doit détecter un jeton littéral de 20+ caractères'
    Assert-True ('Bearer $TOKEN' -notmatch $bearerLiteral) `
        'la regex Bearer doit accepter une variable shell/PowerShell'
    # Un retour en arrière vers l'exemple d'origine doit faire échouer ce cas :
    # la regex ne doit pas se contenter d'un court placeholder.
    $oldDocBlock = 'Invoke-RestMethod -Headers @{ Authorization = "Bearer $($token.Trim())" }' + "`n" + '$token = "A1B2C3D4E5F6A7B8C9D0E1F2A3B4C5D6"'
    Assert-True ($oldDocBlock -notmatch $bearerLiteral) `
        'la regex Bearer doit accepter une interpolation PowerShell (pas un littéral)'
    $oldCurlLiteral = 'curl -H "Authorization: Bearer A1B2C3D4E5F6A7B8C9D0E1F2A3B4C5D6"'
    Assert-True ($oldCurlLiteral -match $bearerLiteral) `
        'la regex Bearer doit détecter un jeton littéral collé dans un exemple curl'
    # Un jeton en prose (hors bloc) doit être détecté par le scan du document.
    $prose = 'Le jeton est A1B2C3D4E5F6A7B8C9D0E1F2A3B4C5D6, garde-le secret.'
    Assert-True ($prose -match $hexLiteral) `
        'la regex hex doit détecter un jeton littéral même en prose' 
}

Test-Case 'Docs secrets - placeholder piège absent de la documentation' {
    # Motif ASCII uniquement : la comparaison ne dépend ni du BOM ni de
    # l'encodage du fichier de test (les accents du texte sont évités).
    $trap = '(?i)jeton\s+affich\S*\s+par\s+la\s+board'
    Assert-True ('…jeton affiché par la board…' -match $trap) `
        'le motif ASCII doit détecter le placeholder piège d''origine'
    foreach ($dir in @('docs', 'ios', 'extension')) {
        $root = Join-Path $script:RepoRoot $dir
        if (-not (Test-Path -LiteralPath $root -PathType Container)) { continue }
        foreach ($file in Get-ChildItem -LiteralPath $root -Recurse -File -Filter '*.md') {
            $text = [IO.File]::ReadAllText($file.FullName, [Text.Encoding]::UTF8)
            Assert-True ($text -notmatch $trap) `
                "le placeholder piège (jeton affiché par la board) est encore présent dans $dir/$($file.Name) : un utilisateur pourrait le remplacer par le vrai jeton et le laisser dans l'historique."
        }
    }
}
