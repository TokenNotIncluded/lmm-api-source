//! OIDC keyring uses the existing permission-checked private-file loader.
use super::*;
use crate::identity::grants::OidcSigningKeys;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Keyring {
    version: u32,
    active_key_id: String,
    signing_key_file: PathBuf,
    #[serde(default)]
    retired_keys: Vec<PublicKeyFile>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicKeyFile {
    id: String,
    public_key_file: PathBuf,
}

pub(super) fn load(path: &Path, isolated: Option<&Path>) -> io::Result<OidcSigningKeys> {
    let ring: Keyring = parse_document(&private_file(path, 16 * 1024)?)?;
    if ring.version != 1 {
        return Err(invalid_config("unsupported OIDC keyring version"));
    }
    let read = |path: &Path| -> io::Result<Vec<u8>> {
        absolute_path(path)?;
        match isolated {
            Some(directory) => isolated_secret(directory, path, 16 * 1024),
            None => private_file(path, 16 * 1024),
        }
    };
    let signing = Zeroizing::new(read(&ring.signing_key_file)?);
    let retired = ring
        .retired_keys
        .into_iter()
        .map(|key| Ok((key.id, read(&key.public_key_file)?)))
        .collect::<io::Result<Vec<_>>>()?;
    OidcSigningKeys::from_pem(&ring.active_key_id, &signing, retired)
        .map_err(|_| invalid_config("invalid OIDC RSA signing keyring"))
}
