use super::error::{ConfigError, invalid};
use hickory_proto::rr::Name;
use std::net::{IpAddr, SocketAddr};

pub(super) fn parse_endpoint(value: &str, field: &str) -> Result<SocketAddr, ConfigError> {
    let endpoint = value.parse::<SocketAddr>().map_err(|error| {
        invalid(
            field,
            format!("adresse IP et port attendus (IPv6 entre crochets) : {error}"),
        )
    })?;
    if endpoint.port() == 0 {
        return Err(invalid(field, "le port doit être compris entre 1 et 65535"));
    }
    Ok(endpoint)
}

pub(super) fn parse_rewrite_name(pattern: &str, field: &str) -> Result<Name, ConfigError> {
    if !pattern.is_ascii() {
        return Err(invalid(
            field,
            "nom ASCII attendu ; utiliser le punycode pour les noms internationalisés",
        ));
    }
    if pattern.contains('\\') {
        return Err(invalid(field, "les échappements DNS ne sont pas acceptés"));
    }
    if pattern.contains('*') {
        match pattern.strip_prefix("*.") {
            Some(suffix) if !suffix.is_empty() && !suffix.contains('*') => {}
            _ => return Err(invalid(field, "wildcard attendu sous la forme *.domaine")),
        }
    }
    let mut name = Name::from_ascii(pattern)
        .map_err(|error| invalid(field, format!("nom DNS invalide : {error}")))?;
    name.set_fqdn(true);
    if name.is_root() {
        return Err(invalid(
            field,
            "le nom ne peut être vide ou désigner la racine",
        ));
    }
    Ok(name.to_lowercase())
}

pub(super) fn parse_upstreams(
    values: &[String],
    listen: SocketAddr,
) -> Result<Vec<SocketAddr>, ConfigError> {
    if values.is_empty() {
        return Err(invalid("dns.upstreams", "au moins un upstream est requis"));
    }
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let field = format!("dns.upstreams[{index}]");
            let endpoint = parse_endpoint(value, &field)?;
            // La forme canonique couvre aussi les adresses IPv4 mappées en IPv6.
            let ip = endpoint.ip().to_canonical();
            if ip.is_unspecified()
                || ip.is_multicast()
                || matches!(ip, IpAddr::V4(address) if address.is_broadcast())
            {
                return Err(invalid(
                    &field,
                    "l'upstream doit être une adresse IP unicast explicite",
                ));
            }
            if endpoint.port() == listen.port()
                && (ip == listen.ip().to_canonical()
                    || (listen.ip().is_unspecified() && ip.is_loopback()))
            {
                return Err(invalid(&field, "l'upstream désigne le serveur lui-même"));
            }
            Ok(endpoint)
        })
        .collect()
}
