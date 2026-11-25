use super::{CoreFile, absolute_path, configuration_file, invalid_config, parse_document};
use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 256 * 1024;

pub(super) fn load(path: &Path) -> io::Result<CoreFile> {
    absolute_path(path)?;
    let meta =
        fs::symlink_metadata(path).map_err(|_| invalid_config("core configuration unavailable"))?;
    let mut files = Vec::new();
    let directory = if meta.is_dir() {
        path.to_owned()
    } else {
        files.push(path.to_owned());
        let mut name = path.as_os_str().to_owned();
        name.push(".d");
        PathBuf::from(name)
    };
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound && !files.is_empty() => {
            return parse_document(&configuration_file(path, MAX_BYTES)?);
        }
        Ok(meta) if meta.is_dir() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o022 != 0 {
                    return Err(invalid_config(
                        "core configuration directory must not be group or world writable",
                    ));
                }
            }
        }
        _ => return Err(invalid_config("invalid core configuration directory")),
    }
    let first_fragment = files.len();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        if !entry.file_name().as_encoded_bytes().starts_with(b".")
            && entry.path().extension() == Some(OsStr::new("toml"))
        {
            if files.len() - first_fragment >= 256 {
                return Err(invalid_config("too many core TOML fragments"));
            }
            files.push(entry.path());
        }
    }
    files[first_fragment..].sort();
    let mut merged = toml::Value::Table(toml::Table::new());
    let mut remaining = MAX_BYTES;
    for file in files {
        let bytes = configuration_file(&file, remaining)?;
        remaining -= bytes.len();
        let table: toml::Table = parse_document(&bytes)?;
        overlay(&mut merged, toml::Value::Table(table));
    }
    merged
        .try_into()
        .map_err(|_| invalid_config("invalid merged core TOML configuration"))
}

fn overlay(base: &mut toml::Value, later: toml::Value) {
    match (base, later) {
        (toml::Value::Table(base), toml::Value::Table(later)) => {
            for (key, value) in later {
                if let Some(old) = base.get_mut(&key) {
                    overlay(old, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, value) => *base = value,
    }
}
