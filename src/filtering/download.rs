use crate::config::FilteringConfig;
use std::{
    io,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

pub(super) fn client(config: &FilteringConfig) -> io::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(config.download_timeout)
        .connect_timeout(config.download_timeout)
        .user_agent(concat!("light-dns/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if let Some(error) = redirect_error(attempt.url(), attempt.previous()) {
                return attempt.error(error);
            }
            attempt.follow()
        }))
        .build()
        .map_err(io::Error::other)
}

pub(super) fn redirect_error(
    url: &reqwest::Url,
    previous: &[reqwest::Url],
) -> Option<&'static str> {
    if previous.len() >= 3 {
        return Some("trop de redirections");
    }
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Some("redirection hors HTTP(S) refusée");
    }
    if previous.iter().any(|url| url.scheme() == "https") && url.scheme() != "https" {
        return Some("redirection HTTPS vers HTTP refusée");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Some("identifiants dans la redirection refusés");
    }
    None
}

pub(super) async fn fetch(client: &reqwest::Client, url: &str, max: usize) -> io::Result<String> {
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|error| io::Error::other(error.without_url()))?
        .error_for_status()
        .map_err(|error| io::Error::other(error.without_url()))?;
    if response
        .content_length()
        .is_some_and(|size| size > max as u64)
    {
        return Err(io::Error::other("liste distante trop volumineuse"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| io::Error::other(error.without_url()))?
    {
        if bytes.len() + chunk.len() > max {
            return Err(io::Error::other("liste distante trop volumineuse"));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(super) async fn read(path: &Path, max: usize) -> io::Result<String> {
    if !fs::metadata(path).await?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "un fichier régulier est requis pour une liste",
        ));
    }
    let file = fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > max {
        return Err(io::Error::other("fichier de filtres trop volumineux"));
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(super) async fn save(dir: &Path, name: &str, url: &str, text: &str) -> io::Result<()> {
    fs::create_dir_all(dir).await?;
    let (temp, mut file) = temporary(dir, name).await?;
    let result = async {
        file.write_all(format!("! light-dns source {url}\n").as_bytes())
            .await?;
        file.write_all(text.as_bytes()).await?;
        file.flush().await?;
        drop(file);
        fs::rename(&temp, dir.join(format!("{name}.txt"))).await
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&temp).await;
    }
    result
}

async fn temporary(dir: &Path, name: &str) -> io::Result<(std::path::PathBuf, fs::File)> {
    // Le PID (notamment 1 dans Docker) et le compteur repartent de zéro après
    // un crash. Ne jamais ouvrir ni supprimer un temporaire déjà présent.
    for _ in 0..32 {
        let path = dir.join(format!(
            "{name}.{}.{}.tmp",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&path).await {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "trop de temporaires de filtres existants ; nouvelle tentative nécessaire",
    ))
}

#[cfg(test)]
#[path = "../../tests/filtering/persistence.rs"]
mod tests;
