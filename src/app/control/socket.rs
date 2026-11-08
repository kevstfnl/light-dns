use std::{
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    net::{UnixListener, UnixStream},
};

pub(super) struct Listener {
    inner: UnixListener,
    path: PathBuf,
    inode: u64,
    device: u64,
    owner: u32,
    // Ne pas supprimer ce fichier : tous les démarrages doivent verrouiller le même inode.
    _lock: fs::File,
}

impl Listener {
    pub async fn bind(path: PathBuf) -> io::Result<Self> {
        // Une paire privée fournit l'UID effectif sans appel système unsafe ni dépendance.
        let (identity_stream, _) = UnixStream::pair()?;
        let owner = identity_stream.peer_cred()?.uid();
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        let lock_path = PathBuf::from(lock_path);
        let lock = open_lock(&lock_path, owner)?;
        let identity = lock.metadata()?;
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if !metadata.file_type().is_socket() || metadata.uid() != identity.uid() {
                return Err(io::Error::other("chemin de contrôle existant protégé"));
            }
            match tokio::time::timeout(Duration::from_millis(250), UnixStream::connect(&path)).await
            {
                Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {}
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "socket de contrôle active ou inaccessible",
                    ));
                }
            }
            let current = fs::symlink_metadata(&path)?;
            if !current.file_type().is_socket()
                || current.ino() != metadata.ino()
                || current.dev() != metadata.dev()
                || current.uid() != metadata.uid()
            {
                return Err(io::Error::other("identité de la socket modifiée"));
            }
            fs::remove_file(&path)?;
        }
        let inner = UnixListener::bind(&path)?;
        let metadata = fs::symlink_metadata(&path)?;
        let listener = Self {
            inner,
            path,
            inode: metadata.ino(),
            device: metadata.dev(),
            owner: metadata.uid(),
            _lock: lock,
        };
        fs::set_permissions(&listener.path, fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    pub async fn accept(&self) -> io::Result<UnixStream> {
        loop {
            let (stream, _) = self.inner.accept().await?;
            let Ok(credentials) = stream.peer_cred() else {
                continue;
            };
            let uid = credentials.uid();
            if uid == self.owner || uid == 0 {
                return Ok(stream);
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.file_type().is_socket()
            && metadata.ino() == self.inode
            && metadata.dev() == self.device
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

// Le dossier et ses ancêtres doivent rester sous le contrôle de l'utilisateur
// du service/root. Le contrôle du parent rejette les erreurs usuelles (/tmp).
fn open_lock(path: &std::path::Path, owner: u32) -> io::Result<fs::File> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let directory = fs::metadata(parent)?;
    if !directory.is_dir()
        || (directory.uid() != owner && directory.uid() != 0)
        || directory.mode() & 0o022 != 0
    {
        return Err(io::Error::other(
            "le dossier de contrôle doit être maîtrisé et non modifiable par le groupe ou les autres",
        ));
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && !metadata.file_type().is_file()
    {
        return Err(io::Error::other("le verrou doit être un fichier régulier"));
    }
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    let identity = lock.metadata()?;
    let linked = fs::symlink_metadata(path)?;
    if !linked.file_type().is_file()
        || identity.uid() != owner
        || identity.mode() & 0o077 != 0
        || identity.nlink() != 1
        || identity.ino() != linked.ino()
        || identity.dev() != linked.dev()
    {
        return Err(io::Error::other("identité du verrou modifiée"));
    }
    lock.try_lock().map_err(io::Error::other)?;
    Ok(lock)
}

#[cfg(test)]
#[path = "../../../tests/app/control/locking.rs"]
mod tests;

pub(super) async fn read_frame(stream: &mut UnixStream, maximum: usize) -> io::Result<Vec<u8>> {
    let mut frame = Vec::with_capacity(maximum);
    while frame.len() < maximum {
        let byte = stream.read_u8().await?;
        frame.push(byte);
        if byte == b'\n' {
            return Ok(frame);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "commande de contrôle trop longue",
    ))
}

pub(super) async fn read_response(stream: &mut UnixStream, maximum: u64) -> io::Result<String> {
    let mut bytes = Vec::new();
    stream.take(maximum + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "réponse de contrôle trop longue",
        ));
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
