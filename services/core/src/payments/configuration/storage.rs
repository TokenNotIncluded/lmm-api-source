//! A bounded private overlay references immutable key files in its own directory.
//! Commit point: atomic overlay rename, after the new key has been fsynced.
use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

pub(super) struct Prepared {
    pub(super) snapshot: Arc<Snapshot>,
    bytes: Vec<u8>,
    secret: Option<(PathBuf, String)>,
}
fn digest(bytes: &[u8]) -> String {
    crate::hex_codec::encode(Sha256::digest(bytes))
}
fn private_folder(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Ok(folder) = fs::symlink_metadata(parent) else {
        return false;
    };
    let Ok(file) = fs::symlink_metadata(path) else {
        return false;
    };
    folder.is_dir()
        && file.is_file()
        && folder.permissions().mode() & 0o777 == 0o700
        && folder.uid() == file.uid()
        && matches!(file.permissions().mode() & 0o777, 0o400 | 0o600)
}
fn state_bytes(path: &Path) -> ConfigurationResult<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(_) => crate::config::private_file(path, 64 * 1024)
            .map(Some)
            .map_err(|_| ConfigurationError::Unavailable),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(ConfigurationError::Unavailable),
    }
}
fn valid_key_name(name: &str, id: i64) -> bool {
    let prefix = format!("epay-key-{id}-");
    name.strip_prefix(&prefix)
        .and_then(|s| s.strip_suffix(".secret"))
        .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn write_new(path: &Path, bytes: &[u8]) -> ConfigurationResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| ConfigurationError::Unavailable)?;
    if file.write_all(bytes).and_then(|_| file.sync_all()).is_err() {
        // Only this successful create_new owns the path it may remove.
        let _ = fs::remove_file(path);
        return Err(ConfigurationError::Unavailable);
    }
    Ok(())
}
struct Lock<'a>(&'a File);
impl Drop for Lock<'_> {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
impl Runtime {
    pub(super) fn load(
        path: PathBuf,
        document: &[u8],
        base: Arc<BTreeMap<i64, Channel>>,
    ) -> ConfigurationResult<Self> {
        // Preserve existing read-only TOML permissions; only a private owned folder
        // enables edits. Overlay/key files always require private regular files.
        let source = crate::config::configuration_file(&path, 64 * 1024)
            .map_err(|_| ConfigurationError::Unavailable)?;
        if source != document {
            return Err(ConfigurationError::Conflict);
        }
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or(ConfigurationError::Invalid)?;
        let state_path = path.with_file_name(format!("{filename}.epay.json"));
        let base_sha256 = digest(document);
        let bytes = state_bytes(&state_path)?;
        let overlay = match &bytes {
            Some(bytes) => {
                serde_json::from_slice::<Overlay>(bytes).map_err(|_| ConfigurationError::Invalid)?
            }
            None => Overlay {
                version: 1,
                base_sha256: base_sha256.clone(),
                bindings: BTreeMap::new(),
            },
        };
        if overlay.version != 1 || overlay.base_sha256 != base_sha256 || overlay.bindings.len() > 16
        {
            return Err(ConfigurationError::Conflict);
        }
        let parent = path.parent().ok_or(ConfigurationError::Invalid)?;
        let mut channels = (*base).clone();
        for (id, value) in &overlay.bindings {
            let channel = channels.get_mut(id).ok_or(ConfigurationError::Invalid)?;
            if !eligible(channel) {
                return Err(ConfigurationError::Denied);
            }
            channel.verification.endpoint = value.endpoint.clone();
            if let Some(notify_url) = &value.notify_url {
                channel.verification.notify_url = crate::config::payment_site_url(notify_url)
                    .map_err(|_| ConfigurationError::Invalid)?;
            }
            if let Some(return_url) = &value.return_url {
                channel.verification.return_url = crate::config::payment_site_url(return_url)
                    .map_err(|_| ConfigurationError::Invalid)?;
            }
            if let Some(name) = &value.secret_file {
                if !valid_key_name(name, *id) {
                    return Err(ConfigurationError::Invalid);
                }
                let secret = crate::config::private_file(&parent.join(name), 4096)
                    .map_err(|_| ConfigurationError::Unavailable)?;
                channel.webhook_secrets = vec![
                    String::from_utf8(secret)
                        .map_err(|_| ConfigurationError::Invalid)?
                        .trim()
                        .to_owned(),
                ];
            }
        }
        let validated = PaymentsStore::compile_channels(channels.into_values().collect())?;
        let snapshot = Snapshot {
            revision: revision(
                &base_sha256,
                bytes.as_deref().unwrap_or_default(),
                &validated,
            ),
            channels: validated,
            overlay,
            disk_sha256: bytes.as_deref().map(digest),
        };
        Ok(Self {
            writable: private_folder(&path),
            path,
            state_path,
            base_sha256,
            live: RwLock::new(Arc::new(snapshot)),
            edits: Mutex::new(()),
        })
    }

    pub(super) fn prepare(
        &self,
        current: &Snapshot,
        id: i64,
        input: EPayUpdate,
    ) -> ConfigurationResult<Prepared> {
        if input.endpoint.len() > 512
            || input.notify_url.len() > 100
            || input.return_url.len() > 100
            || input.secret_key.len() > 4096
        {
            return Err(ConfigurationError::Invalid);
        }
        let mut channels = (*current.channels).clone();
        let channel = channels.get_mut(&id).ok_or(ConfigurationError::NotFound)?;
        channel.verification.endpoint = input.endpoint.trim().to_owned();
        let mut overlay = current.overlay.clone();
        let mut value = overlay.bindings.get(&id).cloned().unwrap_or(Override {
            endpoint: String::new(),
            notify_url: None,
            return_url: None,
            secret_file: None,
        });
        value.endpoint = channel.verification.endpoint.clone();
        // A key-only edit echoes the effective URLs. Preserve their underlying
        // templates (or inheritance from the base file) across site changes.
        for (input, saved, effective) in [
            (
                input.notify_url.trim(),
                &mut value.notify_url,
                &mut channel.verification.notify_url,
            ),
            (
                input.return_url.trim(),
                &mut value.return_url,
                &mut channel.verification.return_url,
            ),
        ] {
            if input != effective.as_str() {
                *effective = crate::config::payment_site_url(input)
                    .map_err(|_| ConfigurationError::Invalid)?;
                *saved = Some(input.to_owned());
            }
        }
        let secret = if input.secret_key.trim().is_empty() {
            None
        } else {
            let key = input.secret_key.trim().to_owned();
            channel.webhook_secrets = vec![key.clone()];
            let name = format!("epay-key-{id}-{:032x}.secret", rand::random::<u128>());
            value.secret_file = Some(name.clone());
            Some((
                self.path
                    .parent()
                    .ok_or(ConfigurationError::Invalid)?
                    .join(name),
                key,
            ))
        };
        let validated = PaymentsStore::compile_channels(channels.into_values().collect())?;
        overlay.bindings.insert(id, value);
        let bytes = serde_json::to_vec_pretty(&overlay).map_err(|_| ConfigurationError::Invalid)?;
        if bytes.len() > 64 * 1024 {
            return Err(ConfigurationError::Invalid);
        }
        let snapshot = Arc::new(Snapshot {
            revision: revision(&self.base_sha256, &bytes, &validated),
            channels: validated,
            overlay,
            disk_sha256: Some(digest(&bytes)),
        });
        Ok(Prepared {
            snapshot,
            bytes,
            secret,
        })
    }

    pub(super) fn persist(
        &self,
        current: &Snapshot,
        prepared: &Prepared,
    ) -> ConfigurationResult<()> {
        if !private_folder(&self.path) {
            return Err(ConfigurationError::Unavailable);
        }
        let parent = self.path.parent().ok_or(ConfigurationError::Invalid)?;
        let lock_path = self.state_path.with_extension("lock");
        match fs::symlink_metadata(&lock_path) {
            Ok(_) => {
                crate::config::private_file(&lock_path, 0)
                    .map_err(|_| ConfigurationError::Unavailable)?;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => (),
            Err(_) => return Err(ConfigurationError::Unavailable),
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock_path)
            .map_err(|_| ConfigurationError::Unavailable)?;
        file.try_lock()
            .map_err(|_| ConfigurationError::Unavailable)?;
        let _lock = Lock(&file);
        let source = crate::config::configuration_file(&self.path, 64 * 1024)
            .map_err(|_| ConfigurationError::Unavailable)?;
        let disk = state_bytes(&self.state_path)?;
        if digest(&source) != self.base_sha256 || disk.as_deref().map(digest) != current.disk_sha256
        {
            return Err(ConfigurationError::Conflict);
        }
        let temporary = parent.join(format!(".epay-state-{:032x}.tmp", rand::random::<u128>()));
        let mut secret_created = false;
        let mut temporary_created = false;
        let before_commit: ConfigurationResult<()> = (|| {
            if let Some((path, key)) = &prepared.secret {
                write_new(path, key.as_bytes())?;
                secret_created = true;
            }
            write_new(&temporary, &prepared.bytes)?;
            temporary_created = true;
            // Persist the immutable key's directory entry before publishing its reference.
            File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(|_| ConfigurationError::Unavailable)?;
            fs::rename(&temporary, &self.state_path)
                .map_err(|_| ConfigurationError::Unavailable)?;
            Ok(())
        })();
        if let Err(error) = before_commit {
            if temporary_created {
                let _ = fs::remove_file(&temporary);
            }
            if secret_created && let Some((path, _)) = &prepared.secret {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
        *self
            .live
            .write()
            .map_err(|_| ConfigurationError::DurabilityUnknown)? = prepared.snapshot.clone();
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| ConfigurationError::DurabilityUnknown)?;
        Ok(())
    }
}
