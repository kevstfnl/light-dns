use std::{
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
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
}

impl Listener {
    pub fn bind(path: PathBuf) -> io::Result<Self> {
        // Ne jamais effacer un fichier ou une socket préexistante, même après un crash.
        let inner = UnixListener::bind(&path)?;
        let metadata = fs::symlink_metadata(&path)?;
        let listener = Self {
            inner,
            path,
            inode: metadata.ino(),
            device: metadata.dev(),
            owner: metadata.uid(),
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
