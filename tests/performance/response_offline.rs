//! Test-only candidate; production always retains its EDNS preflight.
use super::*;
use crate::offline::{self, Result};
use hickory_proto::{
    op::Query,
    rr::{
        Name, RData, Record, RecordType,
        rdata::{A, AAAA, CNAME, SOA, TXT},
    },
};
use std::hint::black_box;

fn wire_name(name: &Name) -> usize {
    1 + name.iter().map(|label| 1 + label.len()).sum::<usize>()
}

fn fits(request: &Request, response: &Message, edns: &Edns) -> Option<bool> {
    if !edns.options().as_ref().is_empty() {
        return None;
    }
    let limit = usize::from(edns.max_payload());
    let mut size = 12usize
        .checked_add(request.queries.as_bytes().len())?
        .checked_add(11)?;
    for record in response.all_sections() {
        let data = match &record.data {
            RData::A(_) => 4,
            RData::AAAA(_) => 16,
            RData::CNAME(name) => wire_name(&name.0),
            RData::SOA(soa) => wire_name(&soa.mname)
                .checked_add(wire_name(&soa.rname))?
                .checked_add(20)?,
            _ => return None,
        };
        size = size
            .checked_add(wire_name(&record.name))?
            .checked_add(10)?
            .checked_add(data)?;
        if size > limit {
            return Some(false);
        }
    }
    Some(size <= limit)
}

fn encode(request: &Request, response: &Message, candidate: bool) -> Vec<u8> {
    let mut response = response.clone();
    let edns = response_edns(request);
    if request.protocol() == Protocol::Udp
        && let Some(edns) = &edns
        && (!candidate || fits(request, &response, edns) != Some(true))
    {
        preserve_opt_on_truncation(request, &mut response, edns);
    }
    let mut bytes = Vec::with_capacity(512);
    let mut encoder = BinEncoder::new(&mut bytes);
    encoder.set_max_size(if request.protocol() == Protocol::Udp {
        edns.as_ref().map_or(512, Edns::max_payload)
    } else {
        u16::MAX
    });
    MessageResponseBuilder::new(&request.queries, edns.as_ref())
        .build(
            response.metadata,
            &response.answers,
            &response.authorities,
            [],
            &response.additionals,
        )
        .destructive_emit(&mut encoder)
        .unwrap(); // In-memory fixtures must encode.
    bytes
}

fn fixture(
    case: &str,
    payload: Option<u16>,
    protocol: Protocol,
    name: &str,
    count: usize,
) -> Result<(Request, Message)> {
    let name = Name::from_ascii(name)?;
    let mut query = Message::query();
    query
        .queries
        .push(Query::query(name.clone(), RecordType::A));
    query.metadata.checking_disabled = true;
    query.edns = payload.map(|size| {
        let mut edns = Edns::new();
        edns.set_max_payload(size).set_dnssec_ok(true);
        edns
    });
    let request = Request::from_bytes(query.to_vec()?, "127.0.0.1:5300".parse()?, protocol)?;
    let mut response = Message::response(query.id, query.op_code);
    response.metadata.authentic_data = true;
    response.metadata.checking_disabled = true;
    let data = match case {
        "aaaa" => RData::AAAA(AAAA("2001:db8::1".parse()?)),
        "cname" => RData::CNAME(CNAME(Name::from_ascii("target.example.")?)),
        "unknown" => RData::TXT(TXT::new(vec!["fallback".into()])),
        _ => RData::A(A("192.0.2.1".parse()?)),
    };
    response.answers = (0..count)
        .map(|_| Record::from_rdata(name.clone(), 60, data.clone()))
        .collect();
    if matches!(case, "nxdomain" | "nodata") {
        response.answers.clear();
        response.metadata.response_code = if case == "nxdomain" {
            ResponseCode::NXDomain
        } else {
            ResponseCode::NoError
        };
        response.authorities.push(Record::from_rdata(
            Name::from_ascii("example.")?,
            60,
            RData::SOA(SOA::new(
                Name::from_ascii("ns.example.")?,
                Name::from_ascii("hostmaster.example.")?,
                1,
                60,
                60,
                60,
                60,
            )),
        ));
    }
    Ok((request, response))
}

#[test]
fn conservative_bound_matches_hickory() -> Result {
    let longest = format!(
        "{}.{}.{}.{}.",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    for name in ["host.example.", longest.as_str()] {
        for protocol in [Protocol::Udp, Protocol::Tcp] {
            for payload in [None, Some(512), Some(800), Some(1232), Some(4096)] {
                for case in ["a", "aaaa", "cname", "nxdomain", "nodata", "unknown"] {
                    for count in [0, 1, 29, 30, 31, 48, 49, 50, 75, 76, 77, 100] {
                        let (request, response) = fixture(case, payload, protocol, name, count)?;
                        let baseline = encode(&request, &response, false);
                        assert_eq!(
                            baseline,
                            encode(&request, &response, true),
                            "{case} {payload:?} {count}"
                        );
                        let parsed = Message::from_vec(&baseline)?;
                        assert_eq!(parsed.edns.is_some(), payload.is_some());
                        if protocol == Protocol::Udp {
                            assert!(
                                baseline.len() <= usize::from(payload.unwrap_or(512).min(1232))
                            );
                            if let Some(edns) = response_edns(&request) {
                                if fits(&request, &response, &edns) == Some(true) {
                                    assert!(!parsed.truncation);
                                }
                                assert_eq!(
                                    parsed.edns.as_ref().unwrap().max_payload(),
                                    edns.max_payload()
                                );
                            }
                        } else {
                            assert!(!parsed.truncation);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "Linux release microbenchmark, no network DNS measurements"]
fn response_serialization() -> Result {
    let mut csv = offline::HEADER.to_owned();
    for (case, payload, protocol, count) in [
        ("a", Some(1232), Protocol::Udp, 1),
        ("aaaa", Some(1232), Protocol::Udp, 1),
        ("cname", Some(1232), Protocol::Udp, 1),
        ("nxdomain", Some(1232), Protocol::Udp, 0),
        ("no_edns", None, Protocol::Udp, 1),
        ("near_limit", Some(512), Protocol::Udp, 29),
        ("truncated", Some(512), Protocol::Udp, 100),
        ("tcp", Some(1232), Protocol::Tcp, 100),
    ] {
        let (request, response) = fixture(case, payload, protocol, "host.example.", count)?;
        for trial in 0..5 {
            for candidate in if trial % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                offline::measure(
                    &mut csv,
                    case,
                    if candidate { "bound" } else { "current" },
                    trial,
                    || encode(black_box(&request), black_box(&response), candidate),
                )?;
            }
        }
    }
    // Separate diagnostic: clone + preflight only, without final encoding.
    let (request, response) = fixture("a", Some(1232), Protocol::Udp, "host.example.", 1)?;
    for trial in 0..5 {
        for candidate in if trial % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            offline::measure(
                &mut csv,
                "a_preflight_only",
                if candidate { "bound" } else { "current" },
                trial,
                || {
                    let mut response = response.clone();
                    let edns = response_edns(&request).unwrap();
                    if !candidate || fits(&request, &response, &edns) != Some(true) {
                        preserve_opt_on_truncation(&request, &mut response, &edns);
                    }
                    black_box(response)
                },
            )?;
        }
    }
    offline::output("response", &csv)
}
