//! Garde SSRF/LFI centrale du pipeline média.
//!
//! Toute URL utilisée pour une résolution ou un téléchargement passe par ce
//! module :
//!
//! 1. allowlist de schémas : seuls `http` et `https` sont acceptés, les URL
//!    `file:`, `gtts:`, `ftp:`, `data:`, les chemins UNC et les URL relatives
//!    sont refusés ;
//! 2. résolution DNS puis refus de toute adresse interne (boucle locale,
//!    RFC1918, lien-local, ULA, multicast, adresses non spécifiées, CGNAT,
//!    documentation, réservées) ;
//! 3. connexion à l'adresse IP validée (le client HTTP est épinglé sur les IP
//!    résolues) pour empêcher le rebinding DNS entre la validation et la
//!    connexion ;
//! 4. redirections suivies manuellement (maximum [`MAX_REDIRECTS`] sauts) avec
//!    revalidation complète de chaque cible.
//!
//! Les helpers sont volontairement sans état : ils reçoivent la
//! [`DownloadConfig`] et renvoient des [`MediaError`] typées à code
//! [`MediaErrorCode::UnsupportedSource`] (source refusée par la politique) ou
//! [`MediaErrorCode::DownloadFailed`] (résolution réseau impossible).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::Client;
use url::{Host, Url};

use crate::config::DownloadConfig;
use crate::error::{MediaError, MediaErrorCode, MediaResult};

/// Nombre maximal de redirections suivies pour une même requête.
pub const MAX_REDIRECTS: usize = 3;

/// Taille maximale d'un corps texte lu par le client HTTP gardé.
pub const MAX_TEXT_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

/// Taille maximale d'un corps d'erreur conservé dans un message.
pub const MAX_ERROR_BODY_BYTES: u64 = 8 * 1024;

/// Cible HTTP validée par la garde.
#[derive(Clone, Debug)]
pub struct ValidatedTarget {
    /// URL validée (schéma `http`/`https`, hôte public).
    pub url: Url,
    /// Hôte normalisé en minuscules (IP littérale ou nom de domaine).
    pub host: String,
    /// Port effectif de l'URL.
    pub port: u16,
    /// Adresses IP autorisées, dans l'ordre de résolution.
    pub addrs: Vec<SocketAddr>,
}

impl ValidatedTarget {
    /// Représentation textuelle de l'URL validée.
    pub fn url_str(&self) -> &str {
        self.url.as_str()
    }
}

/// Construit une erreur « source refusée par la politique média ».
fn unsupported(message: impl Into<String>) -> MediaError {
    MediaError::new(MediaErrorCode::UnsupportedSource, message)
}

/// Valide le schéma et la forme d'une URL média.
///
/// Renvoie l'URL parsée si elle est `http`/`https`, avec un hôte, sans
/// identifiants intégrés et sans chemin UNC.
pub fn parse_media_url(raw: &str) -> MediaResult<Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(unsupported("URL média vide"));
    }
    if trimmed.starts_with("\\\\") || trimmed.starts_with("//") {
        return Err(unsupported(
            "les chemins UNC et les URL relatives sont refusés par la politique média",
        ));
    }
    let parsed = Url::parse(trimmed).map_err(|error| {
        MediaError::with_raw(
            MediaErrorCode::UnsupportedSource,
            format!("URL média invalide : {trimmed}"),
            error.to_string(),
        )
    })?;
    match parsed.scheme() {
        "http" | "https" => {}
        "file" => {
            return Err(unsupported(
                "le schéma file: est refusé pour les sources média (lecture de fichiers locaux)",
            ));
        }
        "gtts" => {
            return Err(unsupported(
                "la synthèse vocale gtts: n'est pas prise en charge par le pipeline média",
            ));
        }
        other => {
            return Err(unsupported(format!(
                "schéma d'URL non autorisé : {other} (attendu : http ou https)"
            )));
        }
    }
    if parsed.host().is_none() {
        return Err(unsupported("URL média sans hôte"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(unsupported(
            "URL média avec identifiants intégrés refusée par la politique média",
        ));
    }
    Ok(parsed)
}

/// Motif de blocage d'une adresse IP interne, ou `None` si l'adresse est
/// publique et peut être contactée.
pub fn ip_block_reason(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(ipv4) => ipv4_block_reason(ipv4),
        IpAddr::V6(ipv6) => ipv6_block_reason(ipv6),
    }
}

fn ipv4_block_reason(ip: Ipv4Addr) -> Option<&'static str> {
    let octets = ip.octets();
    if ip.is_loopback() {
        return Some("boucle locale");
    }
    if ip.is_unspecified() {
        return Some("adresse non spécifiée");
    }
    if ip.is_private() {
        return Some("réseau privé (RFC1918)");
    }
    if ip.is_link_local() {
        return Some("adresse lien-local (métadonnées cloud)");
    }
    if ip.is_multicast() {
        return Some("multicast");
    }
    if ip.is_broadcast() {
        return Some("broadcast");
    }
    if octets[0] == 0 {
        return Some("réseau courant 0.0.0.0/8");
    }
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return Some("plage CGNAT 100.64.0.0/10");
    }
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 2 {
        return Some("adresse de documentation");
    }
    if octets[0] == 198 && (octets[1] == 51 && octets[2] == 100) {
        return Some("adresse de documentation");
    }
    if octets[0] == 203 && octets[1] == 0 && octets[2] == 113 {
        return Some("adresse de documentation");
    }
    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
        return Some("plage de benchmarking 198.18.0.0/15");
    }
    if octets[0] >= 240 {
        return Some("plage réservée 240.0.0.0/4");
    }
    None
}

fn ipv6_block_reason(ip: Ipv6Addr) -> Option<&'static str> {
    if let Some(ipv4) = ipv4_mapped(ip) {
        return ipv4_block_reason(ipv4).map(|reason| match reason {
            "boucle locale" => "boucle locale (IPv4 mappée)",
            "réseau privé (RFC1918)" => "réseau privé IPv4 mappé (RFC1918)",
            _ => reason,
        });
    }
    if ip.is_loopback() {
        return Some("boucle locale");
    }
    if ip.is_unspecified() {
        return Some("adresse non spécifiée");
    }
    if ip.is_multicast() {
        return Some("multicast");
    }
    let segments = ip.segments();
    if segments[0] & 0xfe00 == 0xfc00 {
        return Some("adresse unique locale (ULA fc00::/7)");
    }
    if segments[0] & 0xffc0 == 0xfe80 {
        return Some("adresse lien-local IPv6 (fe80::/10)");
    }
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return Some("adresse de documentation IPv6");
    }
    None
}

/// Extrait l'IPv4 encapsulée dans une adresse IPv4-mappée (`::ffff:a.b.c.d`).
fn ipv4_mapped(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    if segments[..5].iter().all(|segment| *segment == 0) && segments[5] == 0xffff {
        let high = u16::to_be_bytes(segments[6]);
        let low = u16::to_be_bytes(segments[7]);
        return Some(Ipv4Addr::new(high[0], high[1], low[0], low[1]));
    }
    None
}

/// Valide une URL média : schéma, forme, résolution DNS et refus des IP
/// internes.
pub async fn validate_target(raw: &str) -> MediaResult<ValidatedTarget> {
    let url = parse_media_url(raw)?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| unsupported("URL média sans port exploitable"))?;
    let (host, resolved) = match url.host() {
        Some(Host::Ipv4(ip)) => (ip.to_string(), vec![IpAddr::V4(ip)]),
        Some(Host::Ipv6(ip)) => (ip.to_string(), vec![IpAddr::V6(ip)]),
        Some(Host::Domain(domain)) => {
            let domain = domain.to_ascii_lowercase();
            let resolved = resolve_host(&domain, port).await?;
            (domain, resolved)
        }
        None => return Err(unsupported("URL média sans hôte")),
    };
    let mut addrs: Vec<SocketAddr> = Vec::new();
    for ip in resolved {
        if let Some(reason) = ip_block_reason(ip) {
            return Err(MediaError::new(
                MediaErrorCode::UnsupportedSource,
                format!("adresse IP interne refusée ({reason}) : {ip}"),
            ));
        }
        let addr = SocketAddr::new(ip, port);
        if !addrs.contains(&addr) {
            addrs.push(addr);
        }
    }
    if addrs.is_empty() {
        return Err(MediaError::new(
            MediaErrorCode::DownloadFailed,
            format!("aucune adresse IP exploitable pour l'hôte {host}"),
        ));
    }
    Ok(ValidatedTarget {
        url,
        host,
        port,
        addrs,
    })
}

/// Résout les adresses IP d'un hôte (nom de domaine ou IP littérale).
async fn resolve_host(host: &str, port: u16) -> MediaResult<Vec<IpAddr>> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    let resolved = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| {
            MediaError::with_raw(
                MediaErrorCode::DownloadFailed,
                format!("résolution DNS impossible pour l'hôte {host} : {error}"),
                error.to_string(),
            )
        })?;
    let addrs: Vec<IpAddr> = resolved.map(|addr| addr.ip()).collect();
    if addrs.is_empty() {
        return Err(MediaError::new(
            MediaErrorCode::DownloadFailed,
            format!("résolution DNS vide pour l'hôte {host}"),
        ));
    }
    Ok(addrs)
}

/// Construit un client HTTP épinglé sur les adresses IP validées.
///
/// Les redirections sont désactivées (elles sont suivies manuellement par
/// [`crate::http::send_guarded`]) et les proxys d'environnement sont ignorés
/// pour empêcher un rebinding DNS entre validation et connexion.
pub fn build_pinned_client(
    config: &DownloadConfig,
    target: &ValidatedTarget,
) -> MediaResult<Client> {
    let mut builder = Client::builder()
        .user_agent(config.user_agent.clone())
        .timeout(config.timeout)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if target.host.parse::<IpAddr>().is_err() {
        for addr in &target.addrs {
            builder = builder.resolve(&target.host, *addr);
        }
    }
    builder.build().map_err(|error| {
        MediaError::new(
            MediaErrorCode::DownloadFailed,
            format!("construction du client HTTP : {error}"),
        )
    })
}

/// Résout une redirection HTTP relative à l'URL courante.
///
/// Seuls `http` et `https` sont acceptés ; la cible n'est pas encore résolue
/// (elle doit être revalidée par [`validate_target`]).
pub fn resolve_redirect(current: &Url, location: &str) -> MediaResult<Url> {
    let location = location.trim();
    if location.is_empty() {
        return Err(MediaError::new(
            MediaErrorCode::DownloadFailed,
            "redirection HTTP sans destination",
        ));
    }
    if location.starts_with("\\\\") {
        return Err(unsupported("redirection HTTP vers un chemin UNC refusée"));
    }
    let next = current.join(location).map_err(|error| {
        MediaError::with_raw(
            MediaErrorCode::DownloadFailed,
            format!("redirection HTTP invalide : {location}"),
            error.to_string(),
        )
    })?;
    match next.scheme() {
        "http" | "https" => Ok(next),
        other => Err(unsupported(format!(
            "redirection vers un schéma non autorisé : {other}"
        ))),
    }
}

/// Teste si une URL (schéma + hôte) est valide sans attendre de durée de
/// connexion ; utilitaire des tests de politique.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn reason(ip: &str) -> Option<&'static str> {
        ip_block_reason(ip.parse::<IpAddr>().unwrap())
    }

    #[test]
    fn rejects_unsafe_schemes_and_paths() {
        for raw in [
            "file:/etc/passwd",
            "file:C:\\Windows\\win.ini",
            "file:///etc/passwd",
            "gtts:bonjour",
            "ftp://example.com/video.mp4",
            "data:video/mp4;base64,AAAA",
            "javascript:alert(1)",
            "\\\\server\\share\\video.mp4",
            "//server/share/video.mp4",
            "C:\\video.mp4",
            "",
        ] {
            let error = parse_media_url(raw).expect_err(raw);
            assert_eq!(error.code(), MediaErrorCode::UnsupportedSource, "{raw}");
        }
    }

    #[test]
    fn rejects_userinfo() {
        let error =
            parse_media_url("https://user:secret@example.com/video.mp4").expect_err("userinfo");
        assert_eq!(error.code(), MediaErrorCode::UnsupportedSource);
        assert!(error.message().contains("identifiants"));
    }

    #[test]
    fn accepts_public_http_urls() {
        for raw in [
            "https://example.com/video.mp4",
            "http://example.com/video.mp4",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        ] {
            let parsed = parse_media_url(raw).expect(raw);
            assert!(matches!(parsed.scheme(), "http" | "https"));
        }
    }

    #[test]
    fn blocks_private_and_special_ipv4_ranges() {
        let blocked = [
            "127.0.0.1",
            "127.1.2.3",
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.1.10",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "224.0.0.1",
            "255.255.255.255",
            "192.0.2.10",
            "198.51.100.10",
            "203.0.113.10",
            "198.18.0.1",
            "240.0.0.1",
        ];
        for ip in blocked {
            assert!(reason(ip).is_some(), "{ip} devrait être refusée");
        }
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "100.63.255.255",
            "100.128.0.1",
        ] {
            assert!(reason(ip).is_none(), "{ip} devrait être autorisée");
        }
    }

    #[test]
    fn blocks_private_and_special_ipv6_ranges() {
        let blocked = [
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
        ];
        for ip in blocked {
            assert!(reason(ip).is_some(), "{ip} devrait être refusée");
        }
        for ip in ["2606:4700:4700::1111", "2001:4860:4860::8888"] {
            assert!(reason(ip).is_none(), "{ip} devrait être autorisée");
        }
    }

    #[tokio::test]
    async fn refuses_loopback_before_any_connection() {
        let error = validate_target("http://127.0.0.1:9/video.mp4")
            .await
            .expect_err("loopback refusé");
        assert_eq!(error.code(), MediaErrorCode::UnsupportedSource);
        assert!(error.message().contains("boucle locale"), "{error}");
        let error = validate_target("http://[::1]:9/video.mp4")
            .await
            .expect_err("loopback IPv6 refusé");
        assert_eq!(error.code(), MediaErrorCode::UnsupportedSource);
        let error = validate_target("http://169.254.169.254/latest/meta-data")
            .await
            .expect_err("métadonnées cloud refusées");
        assert_eq!(error.code(), MediaErrorCode::UnsupportedSource);
    }

    #[tokio::test]
    async fn accepts_public_ip_literal_without_dns() {
        let target = validate_target("https://93.184.216.34/video.mp4")
            .await
            .expect("IP publique acceptée");
        assert_eq!(target.host, "93.184.216.34");
        assert_eq!(target.port, 443);
        assert_eq!(target.addrs.len(), 1);
    }

    #[tokio::test]
    async fn rejects_redirect_to_internal_target() {
        let current = Url::parse("https://www.tiktok.com/@_/video/123").expect("URL publique");
        let next = resolve_redirect(&current, "http://127.0.0.1:8080/secret").expect("URL résolue");
        let error = validate_target(next.as_str())
            .await
            .expect_err("redirection interne refusée");
        assert_eq!(error.code(), MediaErrorCode::UnsupportedSource);
        assert!(error.message().contains("boucle locale"), "{error}");
    }

    #[test]
    fn resolves_relative_and_rejects_unsafe_redirects() {
        let current = Url::parse("https://example.com/a/b").expect("URL");
        let next = resolve_redirect(&current, "../c").expect("relative");
        assert_eq!(next.as_str(), "https://example.com/c");
        let error = resolve_redirect(&current, "file:///etc/passwd").expect_err("schéma refusé");
        assert_eq!(error.code(), MediaErrorCode::UnsupportedSource);
        let error = resolve_redirect(&current, "").expect_err("Location vide");
        assert_eq!(error.code(), MediaErrorCode::DownloadFailed);
    }

    #[test]
    fn pinned_client_builds_for_public_target() {
        let config = DownloadConfig {
            timeout: Duration::from_secs(5),
            short_url_timeout: Duration::from_secs(3),
            max_size_bytes: 1024,
            max_concurrent_ingestions: 1,
            user_agent: "lc-media-test".to_owned(),
        };
        let target = ValidatedTarget {
            url: Url::parse("https://93.184.216.34/video.mp4").expect("URL"),
            host: "93.184.216.34".to_owned(),
            port: 443,
            addrs: vec![SocketAddr::from(([93, 184, 216, 34], 443))],
        };
        assert!(build_pinned_client(&config, &target).is_ok());
    }
}
