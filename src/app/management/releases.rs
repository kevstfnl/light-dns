use super::{Result, System};
use reqwest::Client;
use semver::Version;
use serde::Deserialize;
use std::{fs, os::unix::fs::DirBuilderExt, path::PathBuf, time::Duration};

const REPO: &str = "https://api.github.com/repos/kevstfnl/light-dns";
const TARGET: &str = "aarch64-unknown-linux-musl";

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    assets: Vec<Asset>,
}

fn version(tag: &str) -> Result<Version> {
    Ok(Version::parse(tag.strip_prefix('v').unwrap_or(tag))?)
}

fn binary_name(tag: &str) -> String {
    format!("light-dns-{tag}-{TARGET}")
}

fn compatible(release: &Release) -> bool {
    !release.draft
        && version(&release.tag_name).is_ok()
        && [
            binary_name(&release.tag_name),
            "light-dns.initd".into(),
            "SHA256SUMS".into(),
        ]
        .iter()
        .all(|name| release.assets.iter().filter(|a| a.name == *name).count() == 1)
}

fn select(releases: Vec<Release>) -> Result<Release> {
    releases
        .into_iter()
        .filter(compatible)
        .max_by_key(|r| version(&r.tag_name).ok())
        .ok_or_else(|| "Aucune release publique compatible (bêta incluse).".into())
}

async fn fetch(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut response = client.get(url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err("Téléchargement trop volumineux.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err("Téléchargement trop volumineux.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn asset_url(release: &Release, name: &str) -> Result<String> {
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or("Asset manquant")?;
    let expected = format!(
        "https://github.com/kevstfnl/light-dns/releases/download/{}/{name}",
        release.tag_name
    );
    if asset.browser_download_url != expected {
        return Err("URL de release inattendue.".into());
    }
    Ok(expected)
}

fn digest<'a>(sums: &'a str, name: &str) -> Result<&'a str> {
    let mut found = None;
    for line in sums.lines() {
        let Some((hash, file)) = line.split_once("  ") else {
            return Err("Format SHA256SUMS invalide.".into());
        };
        if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Somme SHA-256 invalide.".into());
        }
        if file == name {
            if found.is_some() {
                return Err("Somme SHA-256 dupliquée.".into());
            }
            found = Some(hash);
        }
    }
    found.ok_or_else(|| format!("Somme manquante : {name}").into())
}

struct Download(PathBuf);
impl Download {
    fn new() -> Result<Self> {
        for n in 0..100 {
            let path = PathBuf::from("/tmp")
                .join(format!("light-dns-download-{}-{n}", std::process::id()));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        Err("Impossible de créer le répertoire temporaire privé.".into())
    }
}
impl Drop for Download {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
pub(super) fn test_directory() -> Result<PathBuf> {
    let directory = Download::new()?;
    let path = directory.0.clone();
    std::mem::forget(directory);
    Ok(path)
}

pub(super) async fn update(system: &System, check: bool, requested: Option<&str>) -> Result<()> {
    let client = Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("light-dns/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" || attempt.previous().len() >= 5 {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()?;
    let release = if let Some(tag) = requested {
        let normalized = format!("v{}", version(tag)?);
        let release: Release = serde_json::from_slice(
            &fetch(
                &client,
                &format!("{REPO}/releases/tags/{normalized}"),
                4 * 1024 * 1024,
            )
            .await?,
        )?;
        if release.tag_name != normalized || !compatible(&release) {
            return Err("Release demandée incompatible.".into());
        }
        release
    } else {
        let mut releases = Vec::new();
        for page in 1..=10 {
            let current: Vec<Release> = serde_json::from_slice(
                &fetch(
                    &client,
                    &format!("{REPO}/releases?per_page=100&page={page}"),
                    4 * 1024 * 1024,
                )
                .await?,
            )?;
            let complete = current.len() < 100;
            releases.extend(current);
            if complete {
                break;
            }
            if page == 10 {
                return Err("Trop de releases : préciser --version.".into());
            }
        }
        select(releases)?
    };
    let installed = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let available = version(&release.tag_name)?;
    println!("Installée : {installed} ; disponible : {available}");
    if check || (requested.is_none() && available <= installed) {
        return Ok(());
    }
    let directory = Download::new()?;
    let sums =
        String::from_utf8(fetch(&client, &asset_url(&release, "SHA256SUMS")?, 65536).await?)?;
    let name = binary_name(&release.tag_name);
    let mut files = Vec::new();
    for (name, limit) in [
        (name.as_str(), 128 * 1024 * 1024),
        ("light-dns.initd", 65536),
    ] {
        let hash = digest(&sums, name)?;
        let bytes = fetch(&client, &asset_url(&release, name)?, limit).await?;
        let path = directory.0.join(name);
        fs::write(&path, &bytes)?;
        let output = system.output("sha256sum", &[path.to_str().ok_or("Chemin UTF-8 requis")?])?;
        let actual = String::from_utf8(output.stdout)?;
        if !output.status.success()
            || actual
                .split_whitespace()
                .next()
                .is_none_or(|v| !v.eq_ignore_ascii_case(hash))
        {
            return Err(format!("Somme SHA-256 incorrecte : {name}").into());
        }
        files.push(bytes);
    }
    let binary = &files[0];
    if binary.len() < 20 || &binary[..6] != b"\x7fELF\x02\x01" || binary[18..20] != [183, 0] {
        return Err("Binaire ELF AArch64 64 bits requis.".into());
    }
    system.install(binary, &files[1]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn response(raw: &'static [u8], limit: usize) -> Result<Vec<u8>> {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}", listener.local_addr()?);
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            let (mut socket, _) = listener.accept()?;
            socket.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut request = [0; 4096];
            if socket.read(&mut request)? == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "requête HTTP absente",
                ));
            }
            socket.write_all(raw)?;
            Ok(())
        });
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        let result = fetch(&client, &url, limit).await;
        server.join().map_err(|_| "Fixture HTTP interrompue")??;
        result
    }

    #[tokio::test]
    async fn downloads_reject_http_errors_oversize_and_interrupted_bodies() -> Result<()> {
        assert_eq!(
            response(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc", 3).await?,
            b"abc"
        );
        for (raw, limit) in [
            (
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".as_slice(),
                100,
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nabcd".as_slice(),
                3,
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nabc".as_slice(),
                100,
            ),
            (
                b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabcd".as_slice(),
                3,
            ),
        ] {
            assert!(response(raw, limit).await.is_err());
        }
        Ok(())
    }
    fn release(tag: &str, draft: bool) -> Release {
        Release {
            tag_name: tag.into(),
            draft,
            assets: [
                binary_name(tag),
                "light-dns.initd".into(),
                "SHA256SUMS".into(),
            ]
            .into_iter()
            .map(|name| Asset {
                name,
                browser_download_url: String::new(),
            })
            .collect(),
        }
    }
    #[test]
    fn selection_includes_beta_excludes_drafts_and_incomplete_assets() -> Result<()> {
        let mut incomplete = release("v9.0.0", false);
        incomplete.assets.clear();
        let chosen = select(vec![
            release("v0.1.0", false),
            release("v0.2.0-beta.2", false),
            release("v0.2.0-beta.10", false),
            release("v8.0.0", true),
            incomplete,
        ])?;
        assert_eq!(chosen.tag_name, "v0.2.0-beta.10");
        assert!(select(vec![release("invalid", false)]).is_err());
        assert_eq!(version("0.2.0-beta.1")?, version("v0.2.0-beta.1")?);
        assert!(version("../../x").is_err());
        Ok(())
    }
    #[test]
    fn strict_checksums_and_asset_urls() -> Result<()> {
        let hash = "a".repeat(64);
        let sums = format!("{hash}  binary\n");
        assert_eq!(digest(&sums, "binary")?, hash);
        assert!(digest(&sums, "other").is_err());
        assert!(digest(&(sums.clone() + &sums), "binary").is_err());
        assert!(digest("invalid  binary", "binary").is_err());
        assert!(asset_url(&release("v0.2.0-beta.1", false), "SHA256SUMS").is_err());
        Ok(())
    }
}
