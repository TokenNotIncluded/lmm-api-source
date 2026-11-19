use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata},
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub fn private_directory(path: &Path) -> Result<u32, &'static str> {
    let info = fs::symlink_metadata(path).map_err(|_| "private directory unavailable")?;
    if !info.is_dir()
        || info.mode() & 0o077 != 0
        || fs::canonicalize(path).map_err(|_| "private directory unavailable")? != path
    {
        return Err("Extore data and socket directories must be private and canonical");
    }
    controlled_ancestors(
        path.parent().ok_or("invalid private directory")?,
        info.uid(),
    )?;
    Ok(info.uid())
}

fn controlled_ancestors(path: &Path, owner: u32) -> Result<(), &'static str> {
    for directory in path.ancestors() {
        let info = fs::symlink_metadata(directory).map_err(|_| "artifact directory unavailable")?;
        if !info.is_dir()
            || (info.uid() != 0 && info.uid() != owner)
            || (info.mode() & 0o022 != 0 && info.mode() & 0o1000 == 0)
        {
            return Err("plugin paths must have owner-controlled canonical ancestors");
        }
    }
    Ok(())
}

pub fn verify_library(path: &Path, digest: &[u8; 32], owner: u32) -> Result<(), &'static str> {
    let info = fs::symlink_metadata(path).map_err(|_| "plugin artifact unavailable")?;
    controlled_ancestors(path.parent().ok_or("invalid plugin path")?, owner)?;
    if !info.is_file()
        || (info.uid() != 0 && info.uid() != owner)
        || info.mode() & 0o022 != 0
        || fs::canonicalize(path).map_err(|_| "plugin artifact unavailable")? != path
    {
        return Err("plugin artifact must be an owner-controlled regular file");
    }
    let mut file = File::open(path).map_err(|_| "plugin artifact unavailable")?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|_| "plugin artifact read failed")?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    if hash.finalize().as_slice() != digest {
        return Err("plugin artifact does not match its reviewed SHA256");
    }
    Ok(())
}

// Only remove the socket created by this worker, never a replacement endpoint.
pub struct SocketGuard {
    path: PathBuf,
    info: Metadata,
}

impl SocketGuard {
    pub fn new(path: &Path) -> Result<Self, &'static str> {
        let guard = Self {
            path: path.to_owned(),
            info: fs::symlink_metadata(path).map_err(|_| "plugin socket unavailable")?,
        };
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| "could not restrict plugin socket")?;
        Ok(guard)
    }
}
impl Drop for SocketGuard {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|current| {
            current.dev() == self.info.dev() && current.ino() == self.info.ino()
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
