use std::{
    fs, io,
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) struct TestDirectory(pub(crate) PathBuf);

impl TestDirectory {
    pub(crate) fn new() -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "light-dns-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos(),
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    pub(crate) fn run(&self) -> io::Result<Output> {
        Command::new(env!("CARGO_BIN_EXE_light-dns"))
            .current_dir(&self.0)
            .output()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
