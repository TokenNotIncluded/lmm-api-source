use std::{ffi::OsString, path::PathBuf};
use zclap::{Arg, Event};

pub struct Args {
    pub socket: PathBuf,
    pub library: PathBuf,
    pub sha256: [u8; 32],
    pub data_dir: PathBuf,
    pub origin: String,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Args, &'static str> {
    let args: Vec<_> = args.into_iter().collect();
    let definitions = [
        Arg::value("socket"),
        Arg::value("library"),
        Arg::value("sha256"),
        Arg::value("data-dir"),
        Arg::value("origin"),
    ];
    let cli = zclap::Command::new("lmm-extore-host").args(&definitions);
    let mut parser = cli.parse_os(&args);
    let (mut socket, mut library, mut digest, mut data_dir, mut origin) =
        (None, None, None, None, None);
    while let Some(event) = parser.next().map_err(|_| "invalid host arguments")? {
        match event {
            Event::Value {
                arg: "socket",
                value,
            } => socket = Some(PathBuf::from(value)),
            Event::Value {
                arg: "library",
                value,
            } => library = Some(PathBuf::from(value)),
            Event::Value {
                arg: "data-dir",
                value,
            } => data_dir = Some(PathBuf::from(value)),
            Event::Value {
                arg: "origin",
                value,
            } => origin = Some(value.to_str().ok_or("invalid source origin")?.to_owned()),
            Event::Value {
                arg: "sha256",
                value,
            } => {
                if value.as_encoded_bytes().len() != 64 {
                    return Err("invalid reviewed artifact digest");
                }
                let mut hash = [0; 32];
                zhex::decode(value.as_encoded_bytes(), &mut hash)
                    .map_err(|_| "invalid reviewed artifact digest")?;
                digest = Some(hash);
            }
            _ => return Err("invalid host arguments"),
        }
    }
    parser.finish().map_err(|_| "invalid host arguments")?;
    let result = Args {
        socket: socket.ok_or("missing socket")?,
        library: library.ok_or("missing library")?,
        sha256: digest.ok_or("missing reviewed artifact digest")?,
        data_dir: data_dir.ok_or("missing Extore data directory")?,
        origin: origin.ok_or("missing source origin")?,
    };
    if [&result.socket, &result.library, &result.data_dir]
        .iter()
        .any(|p| !p.is_absolute())
    {
        return Err("host paths must be absolute");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_full_digest_and_unique_options() {
        let valid = vec![
            "--socket".into(),
            "/run/extore/catalog.sock".into(),
            "--library".into(),
            "/usr/lib/libextore_zplugin.so".into(),
            "--sha256".into(),
            "a".repeat(64).into(),
            "--data-dir".into(),
            "/var/lib/extore".into(),
            "--origin".into(),
            "https://extore.example".into(),
        ];
        assert!(parse(valid.clone()).is_ok());
        let mut short = valid.clone();
        short[5] = "aa".into();
        assert!(parse(short).is_err());
        let mut duplicate = valid;
        duplicate.extend([OsString::from("--origin"), "https://other.example".into()]);
        assert!(parse(duplicate).is_err());
    }
}
