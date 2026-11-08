//! Tests internes sans socket : la politique est vérifiable même sans réseau.
use super::{
    SharedFilters, Updater, download,
    index::{Decision, FilterIndex, FilterList},
    parser,
};
use crate::config::{FilterSource, FilteringConfig};
use arc_swap::ArcSwap;
use hickory_proto::{
    op::ResponseCode,
    rr::{Name, RData, RecordType},
};
use std::{
    fs, io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn list(text: &str, trusted: bool) -> FilterList {
    FilterList {
        text: text.into(),
        allow: false,
        allow_dnsrewrite: trusted,
    }
}

fn decide(index: &FilterIndex, name: &str) -> Result<Decision> {
    Ok(index.decide(&Name::from_ascii(name)?, RecordType::A))
}

#[test]
fn remote_rewrites_are_inert_without_changing_blocking_semantics() -> Result {
    let index = FilterIndex::compile(
        &[list(
            "||redirect.test^$dnsrewrite=192.0.2.99\n\
         ||blocked.test^\n\
         ||safe.blocked.test^$important\n\
         @@||safe.blocked.test^$important\n\
         ||disabled.test^$important\n\
         ||disabled.test^$important,badfilter\n\
         ||typed.test^$dnstype=AAAA\n\
         ||empty.test^$dnsrewrite=NOERROR;;\n\
         ||alias.test^$dnsrewrite=target.test\n\
         ||rcode.test^$dnsrewrite=REFUSED\n\
         ||v6.test^$dnsrewrite=2001:db8::1\n",
            false,
        )],
        &[],
    )?;
    for name in [
        "redirect.test.",
        "empty.test.",
        "alias.test.",
        "rcode.test.",
        "v6.test.",
        "disabled.test.",
        "typed.test.",
    ] {
        assert!(matches!(decide(&index, name)?, Decision::Pass), "{name}");
    }
    assert!(matches!(decide(&index, "blocked.test.")?, Decision::Block));
    assert!(matches!(
        decide(&index, "safe.blocked.test.")?,
        Decision::Allow
    ));
    assert!(matches!(
        index.decide(&Name::from_ascii("typed.test.")?, RecordType::AAAA),
        Decision::Block
    ));
    Ok(())
}

#[test]
fn untrusted_exceptions_and_badfilter_cannot_disable_trusted_rewrites() -> Result {
    let trusted = "||host.test^$dnsrewrite=192.0.2.1,important\n";
    let untrusted = "@@||host.test^$dnsrewrite\n\
        @@||host.test^$important,dnsrewrite=192.0.2.1\n\
        ||host.test^$important,dnsrewrite=192.0.2.1,badfilter\n\
        ||ordinary.test^$badfilter\n";
    for reversed in [false, true] {
        let mut lists = vec![
            list(trusted, true),
            list(untrusted, false),
            list("ordinary.test\n", true),
        ];
        if reversed {
            lists.reverse();
        }
        let index = FilterIndex::compile(&lists, &[])?;
        assert!(
            matches!(decide(&index, "host.test.")?, Decision::Rewrite { code: ResponseCode::NoError, data } if data.len() == 1)
        );
        assert!(matches!(decide(&index, "ordinary.test.")?, Decision::Pass));
    }
    Ok(())
}

#[test]
fn trusted_lists_keep_rewrite_exceptions_badfilter_and_user_priorities() -> Result {
    let text = "||host.test^$dnsrewrite=192.0.2.1\n\
        ||host.test^$dnsrewrite=2001:db8::1\n\
        ||host.test^$dnsrewrite=192.0.2.2\n\
        @@||host.test^$dnsrewrite=192.0.2.2\n\
        ||gone.test^$dnsrewrite=192.0.2.3,dnstype=A\n\
        ||gone.test^$dnstype=A,badfilter,dnsrewrite=192.0.2.3\n\
        ||excepted.test^$dnsrewrite=192.0.2.4\n\
        @@||excepted.test^$dnsrewrite\n\
        ||excepted.test^$important\n";
    let index = FilterIndex::compile(&[list(text, true)], &[])?;
    assert!(
        matches!(decide(&index, "host.test.")?, Decision::Rewrite { data, .. } if data.len() == 2 && matches!(data[0], RData::A(_)) && matches!(data[1], RData::AAAA(_)))
    );
    assert!(matches!(decide(&index, "gone.test.")?, Decision::Pass));
    assert!(matches!(decide(&index, "excepted.test.")?, Decision::Block));
    let index = FilterIndex::compile(&[list(text, true)], &[Name::from_ascii("host.test.")?])?;
    assert!(matches!(decide(&index, "host.test.")?, Decision::Allow));
    Ok(())
}

#[test]
fn policy_exclusions_are_distinct_from_incompatible_rules() -> Result {
    let text = "||host.test^$dnsrewrite=192.0.2.1\n@@||host.test^$dnsrewrite\n||host.test^$dnsrewrite=192.0.2.1,badfilter\n";
    let parsed = parser::parse(text, false, false)?;
    assert_eq!(parsed.untrusted_rewrites, 3);
    assert_eq!(parsed.ignored, 0);
    assert!(parsed.rules.is_empty() && parsed.simple.is_empty() && parsed.disabled.is_empty());
    assert_eq!(FilterIndex::compile(&[list(text, false)], &[])?.len(), 0);
    assert!(FilterIndex::compile(&[list("||host.test^$unknown\n", false)], &[]).is_err());
    assert!(
        FilterIndex::compile(
            &[list("||host.test^$dnsrewrite=NOERROR;A;invalid\n", false)],
            &[]
        )
        .is_err()
    );
    // Une valeur TXT ne devient pas un modificateur par une recherche de sous-chaîne.
    let parsed = parser::parse(
        "||txt.test^$dnsrewrite=NOERROR;TXT;\"dnsrewrite\\,badfilter\"",
        false,
        true,
    )?;
    assert_eq!(parsed.untrusted_rewrites, 0);
    assert_eq!(parsed.rules.len(), 1);
    Ok(())
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> io::Result<Self> {
        static ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "light-dns-trust-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn cached_remote_trust_is_recomputed_without_rewriting_disk() -> Result {
    let dir = Directory::new()?;
    let url = "https://lists.invalid/filter.txt";
    let raw =
        format!("! light-dns source {url}\n||host.test^$dnsrewrite=192.0.2.1\nblocked.test\n");
    fs::write(dir.0.join("remote.txt"), &raw)?;
    let local = dir.0.join("local.txt");
    fs::write(&local, "||local.test^$dnsrewrite=192.0.2.2\n")?;
    for trusted in [false, true, false] {
        let snapshot: SharedFilters = Arc::new(ArcSwap::from_pointee(FilterIndex::default()));
        let config = FilteringConfig {
            sources: vec![FilterSource {
                name: "remote".into(),
                url: url.into(),
                allow_dnsrewrite: trusted,
            }],
            cache_dir: dir.0.clone(),
            blocklists: vec![local.clone()],
            ..FilteringConfig::default()
        };
        let _updater = Updater::prepare(config, snapshot.clone()).await?;
        let index = snapshot.load();
        assert_eq!(
            matches!(decide(&index, "host.test.")?, Decision::Rewrite { .. }),
            trusted
        );
        assert!(matches!(decide(&index, "blocked.test.")?, Decision::Block));
        assert!(matches!(
            decide(&index, "local.test.")?,
            Decision::Rewrite { .. }
        ));
        assert_eq!(fs::read_to_string(dir.0.join("remote.txt"))?, raw);
    }
    Ok(())
}

#[tokio::test]
async fn programmatic_sources_also_require_https_for_trust() -> Result {
    let snapshot = Arc::new(ArcSwap::from_pointee(FilterIndex::default()));
    let config = FilteringConfig {
        sources: vec![FilterSource {
            name: "remote".into(),
            url: "http://localhost/list".into(),
            allow_dnsrewrite: true,
        }],
        ..FilteringConfig::default()
    };
    assert!(Updater::prepare(config, snapshot).await.is_err());
    Ok(())
}

#[test]
fn redirects_keep_https_trust_and_are_bounded() -> Result {
    let https = reqwest::Url::parse("https://lists.test/list")?;
    let http = reqwest::Url::parse("http://lists.test/list")?;
    assert!(download::redirect_error(&https, std::slice::from_ref(&http)).is_none());
    assert!(download::redirect_error(&http, std::slice::from_ref(&https)).is_some());
    assert!(download::redirect_error(&http, &[https.clone(), http.clone()]).is_some());
    assert!(
        download::redirect_error(&https, &[https.clone(), https.clone(), https.clone()]).is_some()
    );
    for url in [
        "file:///tmp/list",
        "ftp://lists.test/list",
        "https://user:password@lists.test/list",
    ] {
        assert!(download::redirect_error(&reqwest::Url::parse(url)?, &[]).is_some());
    }
    Ok(())
}
