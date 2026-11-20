<#
.SYNOPSIS
    Obtient un refresh token OAuth pour l'API Chrome Web Store.

.DESCRIPTION
    Ouvre le navigateur sur l'ecran de consentement Google, ecoute la redirection
    sur http://127.0.0.1:<Port>/ et echange le code recu contre un refresh token
    a mettre dans le secret GitHub CWS_REFRESH_TOKEN.

    Le client OAuth doit etre de type "Desktop app" et l'API Chrome Web Store
    activee dans le projet : voir docs/store/README.md.

.PARAMETER ClientId
    Identifiant du client OAuth (Google Cloud > Credentials).

.PARAMETER ClientSecret
    Secret du meme client OAuth.

.PARAMETER Port
    Port local qui recoit la redirection. 8123 par defaut.

.EXAMPLE
    pwsh -NoProfile -File scripts/webstore-refresh-token.ps1 `
      -ClientId "1234567890-abc.apps.googleusercontent.com" `
      -ClientSecret "GOCSPX-..."
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ClientId,

    [Parameter(Mandatory = $true)]
    [string]$ClientSecret,

    [ValidateRange(1024, 65535)]
    [int]$Port = 8123,

    [string]$Scope = 'https://www.googleapis.com/auth/chromewebstore',

    [switch]$NoBrowser,

    [int]$TimeoutSeconds = 300
)

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$redirectUri = "http://127.0.0.1:$Port/"
$authUrl = 'https://accounts.google.com/o/oauth2/v2/auth?' + (@(
        "client_id=$([Uri]::EscapeDataString($ClientId))",
        "redirect_uri=$([Uri]::EscapeDataString($redirectUri))",
        'response_type=code',
        "scope=$([Uri]::EscapeDataString($Scope))",
        'access_type=offline',
        'prompt=consent'
    ) -join '&')

$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $Port)
try {
    $listener.Start()
}
catch {
    throw "impossible d'ecouter sur 127.0.0.1:$Port ($($_.Exception.Message)) ; choisir un autre -Port"
}

$page = '<!doctype html><meta charset="utf-8"><title>LiveChat</title>' +
'<body style="font:16px system-ui;background:#111;color:#eee;display:grid;place-items:center;height:100vh">' +
'<p>Autorisation recue : le terminal affiche le refresh token, cet onglet peut etre ferme.</p>'
$pageBytes = [Text.Encoding]::UTF8.GetBytes($page)
$responseHead = "HTTP/1.1 200 OK`r`nContent-Type: text/html; charset=utf-8`r`n" +
"Content-Length: $($pageBytes.Length)`r`nConnection: close`r`n`r`n"

Write-Host "Consentement Google : coller cette URL dans le navigateur :`n$authUrl`n"
if (-not $NoBrowser) { Start-Process $authUrl | Out-Null }

$code = $null
$errorCode = $null
$deadline = (Get-Date).AddSeconds($TimeoutSeconds)

try {
    while ((Get-Date) -lt $deadline) {
        if (-not $listener.Pending()) {
            Start-Sleep -Milliseconds 250
            continue
        }

        $client = $listener.AcceptTcpClient()
        try {
            $stream = $client.GetStream()
            $reader = [System.IO.StreamReader]::new($stream)
            $requestLine = $reader.ReadLine()
            $null = $reader.ReadLine()
            $target = ''

            if ($requestLine -and $requestLine -match '^GET\s+(\S+)\s+HTTP/') {
                $target = $Matches[1]
            }

            $query = @{}
            $questionMark = $target.IndexOf('?')
            if ($questionMark -ge 0) {
                foreach ($pair in $target.Substring($questionMark + 1).Split('&')) {
                    if (-not $pair) { continue }
                    $parts = $pair.Split('=', 2)
                    $key = [Uri]::UnescapeDataString($parts[0])
                    $value = if ($parts.Length -gt 1) { [Uri]::UnescapeDataString($parts[1].Replace('+', ' ')) } else { '' }
                    $query[$key] = $value
                }
            }

            if ($query.ContainsKey('code')) { $code = $query['code'] }
            elseif ($query.ContainsKey('error')) { $errorCode = $query['error'] }

            $headBytes = [Text.Encoding]::ASCII.GetBytes($responseHead)
            $stream.Write($headBytes, 0, $headBytes.Length)
            $stream.Write($pageBytes, 0, $pageBytes.Length)
            $stream.Flush()

            if ($code -or $errorCode) { break }
        }
        finally {
            $client.Close()
        }
    }
}
finally {
    $listener.Stop()
}

if ($errorCode) { throw "consentement refuse ($errorCode)" }
if (-not $code) { throw "aucune redirection recue en $TimeoutSeconds s" }

Write-Host "Echange du code contre un refresh token..."
try {
    $token = Invoke-RestMethod -Method Post -Uri 'https://oauth2.googleapis.com/token' -Body @{
        code          = $code
        client_id     = $ClientId
        client_secret = $ClientSecret
        redirect_uri  = $redirectUri
        grant_type    = 'authorization_code'
    }
}
catch {
    # PowerShell 5.1 ne remplit pas ErrorDetails : lire le corps de la reponse.
    $detail = ''
    if ($_.ErrorDetails -and $_.ErrorDetails.Message) {
        $detail = $_.ErrorDetails.Message
    }
    elseif ($_.Exception.Response) {
        $responseStream = $_.Exception.Response.GetResponseStream()
        if ($responseStream) {
            $bodyReader = New-Object System.IO.StreamReader($responseStream)
            $detail = $bodyReader.ReadToEnd()
            $bodyReader.Dispose()
        }
    }
    throw "echange du code refuse : $detail"
}

if (-not $token.refresh_token) {
    throw "reponse sans refresh_token : revoquer l'acces de l'application dans le compte Google puis relancer"
}

Write-Host ''
Write-Host 'Refresh token (secret CWS_REFRESH_TOKEN) :'
Write-Host ''
Write-Host "  $($token.refresh_token)"
Write-Host ''
Write-Host 'Ne pas le coller dans une conversation : utiliser la page des secrets du depot'
Write-Host '(Settings > Secrets and variables > Actions), ou :'
Write-Host ''
Write-Host "  gh secret set CWS_REFRESH_TOKEN --body `"<refresh token>`""
Write-Host ''
