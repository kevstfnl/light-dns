use super::*;
use std::{path::PathBuf, sync::atomic::AtomicU64};

struct Directory(PathBuf);
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
impl Directory {
    fn new() -> io::Result<Self> {
        static ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "light-dns-persistence-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn interrupted_temporary_is_preserved_and_does_not_prevent_saving() -> io::Result<()> {
    let _serial = SERIAL.lock().await;
    let dir = Directory::new()?;
    let url = "https://lists.test/list";
    let old = format!("! light-dns source {url}\nold.test\n");
    std::fs::write(dir.0.join("remote.txt"), &old)?;
    // Simule un processus précédent ayant le même PID et compteur (Docker PID 1).
    let id = TEMP_ID.load(Ordering::Relaxed);
    let abandoned = dir
        .0
        .join(format!("remote.{}.{}.tmp", std::process::id(), id));
    std::fs::write(&abandoned, "incomplete")?;
    save(&dir.0, "remote", url, "new.test\n").await?;
    assert_eq!(std::fs::read_to_string(&abandoned)?, "incomplete");
    assert_eq!(
        std::fs::read_to_string(dir.0.join("remote.txt"))?,
        format!("! light-dns source {url}\nnew.test\n")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.0.join("remote.txt"))?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    Ok(())
}

#[tokio::test]
async fn failed_replace_preserves_destination_and_cleans_own_temporary() -> io::Result<()> {
    let _serial = SERIAL.lock().await;
    let dir = Directory::new()?;
    let destination = dir.0.join("remote.txt");
    std::fs::create_dir(&destination)?;
    std::fs::write(destination.join("keep"), "keep")?;
    assert!(
        save(&dir.0, "remote", "https://lists.test/list", "new.test\n")
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(destination.join("keep"))?, "keep");
    assert_eq!(std::fs::read_dir(&dir.0)?.count(), 1);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn preexisting_symlink_temporary_never_changes_its_target() -> io::Result<()> {
    let _serial = SERIAL.lock().await;
    let dir = Directory::new()?;
    let target = dir.0.join("protected");
    std::fs::write(&target, "keep")?;
    let id = TEMP_ID.load(Ordering::Relaxed);
    let abandoned = dir
        .0
        .join(format!("remote.{}.{}.tmp", std::process::id(), id));
    std::os::unix::fs::symlink(&target, &abandoned)?;
    save(&dir.0, "remote", "https://lists.test/list", "new.test\n").await?;
    assert_eq!(std::fs::read_to_string(target)?, "keep");
    assert!(
        std::fs::symlink_metadata(abandoned)?
            .file_type()
            .is_symlink()
    );
    Ok(())
}
