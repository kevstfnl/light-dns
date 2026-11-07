use hickory_proto::{
    op::{Message, Metadata, Query, ResponseCode},
    rr::{
        DNSClass, Name, RData, Record, RecordType,
        rdata::{A, AAAA},
    },
};
use std::{collections::HashMap, net::IpAddr};

const LOCAL_TTL: u32 = 60;

#[derive(Default)]
struct Node {
    address: Option<IpAddr>,
    wildcard: Option<IpAddr>,
    managed_zone: bool,
}

pub(super) struct Rewrites {
    nodes: HashMap<Name, Node>,
}

enum Lookup {
    Address(IpAddr),
    NoData,
    NxDomain,
}

impl Rewrites {
    pub(super) fn new(rules: HashMap<Name, IpAddr>) -> Self {
        let mut nodes: HashMap<Name, Node> = HashMap::with_capacity(rules.len());
        for (name, address) in rules {
            nodes.entry(name.clone()).or_default().address = Some(address);
            if name.is_wildcard() {
                let parent = nodes.entry(name.base_name()).or_default();
                parent.wildcard = Some(address);
                parent.managed_zone = true;
            }
            // Les parents sans enregistrement existent aussi dans l'arbre DNS.
            let mut parent = name.base_name();
            while !parent.is_root() {
                nodes.entry(parent.clone()).or_default();
                parent = parent.base_name();
            }
        }
        Self { nodes }
    }

    pub(super) fn resolve(&self, query: &Query, request: &Metadata) -> Option<Message> {
        let lookup = self.lookup(query.name())?;
        let mut response = Message::response(request.id, request.op_code);
        response.metadata = Metadata::response_from_request(request);
        response.metadata.recursion_available = true;
        if query.query_class() != DNSClass::IN {
            response.metadata.response_code = ResponseCode::Refused;
            return Some(response);
        }
        match lookup {
            Lookup::Address(address) => {
                let data = match (address, query.query_type()) {
                    (IpAddr::V4(ip), RecordType::A | RecordType::ANY) => Some(RData::A(A(ip))),
                    (IpAddr::V6(ip), RecordType::AAAA | RecordType::ANY) => {
                        Some(RData::AAAA(AAAA(ip)))
                    }
                    _ => None,
                };
                if let Some(data) = data {
                    response.answers.push(Record::from_rdata(
                        query.name().clone(),
                        LOCAL_TTL,
                        data,
                    ));
                }
            }
            Lookup::NoData => {}
            Lookup::NxDomain => response.metadata.response_code = ResponseCode::NXDomain,
        }
        Some(response)
    }

    fn lookup(&self, name: &Name) -> Option<Lookup> {
        if self.nodes.is_empty() {
            return None;
        }
        let node = self.nodes.get(name);
        if let Some(address) = node.and_then(|node| node.address) {
            return Some(Lookup::Address(address));
        }
        let mut closest = node;
        let mut candidate = name.clone();
        while !candidate.is_root() {
            if let Some(ancestor) = self.nodes.get(&candidate) {
                closest = closest.or(Some(ancestor));
                if ancestor.managed_zone {
                    return Some(if node.is_some() {
                        Lookup::NoData
                    } else if let Some(address) = closest.and_then(|node| node.wildcard) {
                        Lookup::Address(address)
                    } else {
                        Lookup::NxDomain
                    });
                }
            }
            // RFC 4592 : seul le wildcard de l'ancêtre existant le plus proche
            // peut synthétiser une réponse ; aucun repli sur un wildcard plus haut.
            candidate = candidate.base_name();
        }
        None
    }
}
