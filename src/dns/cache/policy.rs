use hickory_proto::{
    op::{Message, Query, ResponseCode},
    rr::{DNSClass, RData, RecordType},
};
use std::time::Duration;

pub(super) fn prepare(
    response: &Message,
    query: &Query,
    cap: Duration,
) -> Option<(Message, Duration)> {
    if query.query_class() != DNSClass::IN
        || response.truncation
        || response.signature.is_some()
        || !matches!(
            response.response_code,
            ResponseCode::NoError | ResponseCode::NXDomain
        )
    {
        return None;
    }
    let mut ttl = cap.as_secs().min(u32::MAX as u64) as u32;
    for record in response.all_sections() {
        // Les signatures nécessitent aussi des échéances absolues ; elles restent hors cache.
        if record.ttl == 0
            || record.ttl > i32::MAX as u32
            || record.dns_class != DNSClass::IN
            || matches!(
                record.data.record_type(),
                RecordType::RRSIG | RecordType::SIG
            )
        {
            return None;
        }
        ttl = ttl.min(record.ttl);
    }
    let positive = response.response_code == ResponseCode::NoError
        && response.answers.iter().any(|record| {
            record.dns_class == query.query_class()
                && (query.query_type() == RecordType::ANY
                    || record.data.record_type() == query.query_type())
        });
    if !positive {
        // Une référence NS seule ou un SOA d'une zone sans rapport n'est pas un NODATA fiable.
        let negative_ttl = response
            .authorities
            .iter()
            .filter_map(|record| match &record.data {
                RData::SOA(soa) if record.name.zone_of(query.name()) => {
                    Some(record.ttl.min(soa.minimum))
                }
                _ => None,
            })
            .min()?;
        ttl = ttl.min(negative_ttl);
    }
    if ttl == 0 {
        return None;
    }
    let mut message = response.clone();
    message.metadata.id = 0;
    message.metadata.authoritative = false;
    message.edns = None;
    if !positive {
        for record in &mut message.authorities {
            if let RData::SOA(soa) = &record.data {
                record.ttl = record.ttl.min(soa.minimum);
            }
        }
    }
    Some((message, Duration::from_secs(ttl.into())))
}
