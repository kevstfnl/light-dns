use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new() -> io::Result<Self> {
        static ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "light-dns-lock-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(Self(path))
    }
    fn owner(&self) -> io::Result<u32> {
        Ok(fs::metadata(&self.0)?.uid())
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn lock_excludes_another_open_and_survives_repeated_restarts() -> io::Result<()> {
    let dir = Directory::new()?;
    let path = dir.0.join("control.sock.lock");
    let mut inode = None;
    for _ in 0..4 {
        let lock = open_lock(&path, dir.owner()?)?;
        let identity = lock.metadata()?;
        assert_eq!(identity.mode() & 0o777, 0o600);
        assert_eq!(*inode.get_or_insert(identity.ino()), identity.ino());
        assert!(open_lock(&path, dir.owner()?).is_err());
        drop(lock);
        assert!(path.is_file());
    }
    Ok(())
}

#[test]
fn lock_preserves_links_and_rejects_permissive_files_and_directories() -> io::Result<()> {
    let dir = Directory::new()?;
    let target = dir.0.join("protected");
    let path = dir.0.join("control.sock.lock");
    fs::write(&target, "keep")?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600))?;
    std::os::unix::fs::symlink(&target, &path)?;
    assert!(open_lock(&path, dir.owner()?).is_err());
    fs::remove_file(&path)?;
    fs::hard_link(&target, &path)?;
    assert!(open_lock(&path, dir.owner()?).is_err());
    assert_eq!(fs::read_to_string(&target)?, "keep");
    fs::remove_file(&path)?;
    fs::write(&path, "keep")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
    assert!(open_lock(&path, dir.owner()?).is_err());
    assert_eq!(fs::read_to_string(&path)?, "keep");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o777))?;
    assert!(open_lock(&path, dir.owner()?).is_err());
    fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[test]
fn lock_rejects_special_files_without_opening_them() -> io::Result<()> {
    let dir = Directory::new()?;
    assert!(open_lock(std::path::Path::new("/dev/null"), dir.owner()?).is_err());
    assert!(open_lock(&dir.0, dir.owner()?).is_err());
    Ok(())
}
