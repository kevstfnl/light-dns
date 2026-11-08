use hickory_proto::{
    op::ResponseCode,
    rr::{Name, RData, RecordType},
    serialize::txt::Parser,
};
use std::str::FromStr;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum Rewrite {
    All,
    Value {
        code: ResponseCode,
        data: Option<RData>,
    },
}

pub(super) fn parse(value: &str) -> Option<Rewrite> {
    let (code, kind, value) = if let Some((code, rest)) = value.split_once(';') {
        let (kind, value) = rest.split_once(';')?;
        (rcode(code)?, kind.to_owned(), value.to_owned())
    } else if let Some(code) = rcode(value) {
        (code, String::new(), String::new())
    } else if let Ok(ip) = value.parse::<std::net::IpAddr>() {
        (
            ResponseCode::NoError,
            if ip.is_ipv4() { "A" } else { "AAAA" }.into(),
            ip.to_string(),
        )
    } else {
        Name::from_ascii(value).ok()?;
        (ResponseCode::NoError, "CNAME".into(), value.to_owned())
    };
    if kind.is_empty() && value.is_empty() {
        return Some(Rewrite::Value { code, data: None });
    }
    if code != ResponseCode::NoError
        || !matches!(
            RecordType::from_str(&kind).ok()?,
            RecordType::A
                | RecordType::AAAA
                | RecordType::CNAME
                | RecordType::PTR
                | RecordType::MX
                | RecordType::TXT
                | RecordType::SRV
                | RecordType::HTTPS
                | RecordType::SVCB
        )
        || value.contains(['\n', '\r'])
    {
        return None;
    }
    // Un seul RR synthétique ; le parseur Hickory valide également les paramètres SVCB/HTTPS.
    let text = format!(
        "rewrite.invalid. 60 IN {kind} {}\n",
        value.replace("\\,", ",")
    );
    let (_, sets) = Parser::new(text, None, Some(Name::root())).parse().ok()?;
    if sets.len() != 1 {
        return None;
    }
    let set = sets.values().next()?;
    if set.records_count() != 1 {
        return None;
    }
    let data = set.records_without_rrsigs().next()?.data.clone();
    Some(Rewrite::Value {
        code,
        data: Some(data),
    })
}

fn rcode(value: &str) -> Option<ResponseCode> {
    Some(match value {
        "NOERROR" => ResponseCode::NoError,
        "FORMERR" => ResponseCode::FormErr,
        "SERVFAIL" => ResponseCode::ServFail,
        "NXDOMAIN" => ResponseCode::NXDomain,
        "NOTIMP" => ResponseCode::NotImp,
        "REFUSED" => ResponseCode::Refused,
        _ => return None,
    })
}
