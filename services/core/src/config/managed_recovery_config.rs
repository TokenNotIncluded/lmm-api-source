//! Managed recovery key isolation, using the actual configured key-file formats.
use super::{
    FederatedKeyringFile, MfaKeyringFile, ModelConfig, PaymentConfig, ReceiptAdaptersConfig,
    configuration_file, email_config, invalid_config, isolated_secret, parse_document,
    private_file, validate_isolated_secret_directory,
};
use crate::{
    crypto_key_material::{KeyPurpose, hmac_sha256_uses},
    identity::grants::ManagedRecoveryKeys,
    tool_market::receipt_verifier::ReceiptVerifier,
};
use sha2::{Digest, Sha256};
use std::{ffi::OsString, io, path::Path, path::PathBuf};
use zeroize::Zeroizing;

fn kdf_materials(
    path: &Path,
    purpose: KeyPurpose,
    materials: &mut Vec<Zeroizing<Vec<u8>>>,
) -> io::Result<()> {
    let raw = Zeroizing::new(private_file(path, 32)?);
    if raw.len() != 32 {
        return Err(invalid_config(
            "host encryption keys require raw 32-byte files",
        ));
    }
    let derived = purpose
        .derive(&raw)
        .map_err(|_| invalid_config("host key derivation is unavailable"))?;
    materials.push(raw);
    materials.push(Zeroizing::new(derived.to_vec()));
    Ok(())
}

/// This accepts the same resolved settings reader as the host. Unconfigured
/// domains are never opened, generated, or initialized for comparison.
pub(super) fn load(
    setting: impl Fn(&'static str) -> Option<OsString>,
    isolated_directory: Option<&Path>,
) -> io::Result<Option<ManagedRecoveryKeys>> {
    let Some(path) = setting("LMM_CORE_OAUTH2_MANAGED_REFRESH_KEYRING_FILE") else {
        return Ok(None);
    };
    if let Some(directory) = isolated_directory {
        validate_isolated_secret_directory(directory)?;
    }
    let read_managed = |path: &Path, maximum| {
        if let Some(directory) = isolated_directory {
            isolated_secret(directory, path, maximum)
        } else {
            private_file(path, maximum)
        }
    };
    let ring: MfaKeyringFile = parse_document(&read_managed(Path::new(&path), 16 * 1024)?)?;
    if ring.version != 1 || !(1..=8).contains(&ring.keys.len()) {
        return Err(invalid_config("invalid managed recovery keyring"));
    }

    // Compare bytes rather than filenames: copies, hardlinks and aliases cannot
    // turn a session, identity or signing key into a dedicated recovery key.
    let mut other_files: Vec<(PathBuf, usize)> = Vec::new();
    let mut others = Vec::new();
    let mut hmac_keys = Vec::new();
    if let Some(path) = setting("LMM_CORE_API_KEY_ENCRYPTION_KEY_FILE") {
        kdf_materials(Path::new(&path), KeyPurpose::ApiVault, &mut others)?;
    }
    if let Some(path) = setting("LMM_CORE_MFA_KEYRING_FILE") {
        let ring: MfaKeyringFile = parse_document(&private_file(Path::new(&path), 16 * 1024)?)?;
        if ring.version != 1 || !(1..=8).contains(&ring.keys.len()) {
            return Err(invalid_config("invalid MFA keyring configuration"));
        }
        for key in ring.keys {
            kdf_materials(&key.key_file, KeyPurpose::MfaVault, &mut others)?;
        }
    }
    if let Some(path) = setting("LMM_CORE_FEDERATED_AUTH_KEYRING_FILE") {
        let ring: FederatedKeyringFile =
            parse_document(&private_file(Path::new(&path), 16 * 1024)?)?;
        if ring.version != 1 || !(1..=8).contains(&ring.keys.len()) {
            return Err(invalid_config("invalid federated authentication keyring"));
        }
        for key in ring.keys {
            kdf_materials(&key.key_file, KeyPurpose::FederatedVault, &mut others)?;
        }
        kdf_materials(
            &ring.replay_key_file,
            KeyPurpose::FederatedReplay,
            &mut others,
        )?;
    }
    if let Some(path) = setting("LMM_CORE_EMAIL_CONFIG_FILE") {
        other_files.extend(
            email_config::cryptographic_key_files(Path::new(&path))?
                .into_iter()
                .map(|path| (path, 32)),
        );
    }
    others.extend(
        other_files
            .into_iter()
            .map(|(path, maximum)| private_file(&path, maximum).map(Zeroizing::new))
            .collect::<io::Result<Vec<_>>>()?,
    );
    if let Some(path) = setting("LMM_CORE_MODEL_CONFIG_FILE") {
        let model: ModelConfig =
            parse_document(&configuration_file(Path::new(&path), 16 * 1024 * 1024)?)?;
        if let Some(path) = model.channel_secret_key_file {
            kdf_materials(&path, KeyPurpose::ChannelVault, &mut others)?;
        }
    }
    let receipt_verifier =
        if let Some(path) = setting("LMM_CORE_TOOL_MARKET_RECEIPT_ADAPTERS_CONFIG_FILE") {
            let config: ReceiptAdaptersConfig =
                parse_document(&configuration_file(Path::new(&path), 256 * 1024)?)?;
            Some(
                ReceiptVerifier::from_adapters(config.adapters)
                    .map_err(|_| invalid_config("invalid Core supplier receipt adapters"))?,
            )
        } else {
            None
        };
    if let Some(path) = setting("LMM_CORE_PAYMENTS_CONFIG_FILE") {
        let config: PaymentConfig =
            parse_document(&configuration_file(Path::new(&path), 64 * 1024)?)?;
        config.validate()?;
        let channels: Vec<_> = match config {
            PaymentConfig::Legacy(config) => config
                .channels
                .into_iter()
                .map(|channel| ("stripe".to_owned(), channel.webhook_secret_file))
                .collect(),
            PaymentConfig::Gateways(config) => config
                .channels
                .into_iter()
                .map(|channel| (channel.provider, channel.webhook_secret_file))
                .collect(),
        };
        for (provider, path) in channels {
            // Waffo/Pancake verification files are RSA public keys, and their
            // inquiry private keys are not configured 32-byte symmetric keys.
            if !matches!(provider.as_str(), "stripe" | "creem" | "coinbase" | "epay") {
                continue;
            }
            let raw = Zeroizing::new(
                String::from_utf8(private_file(&path, 4096)?)
                    .map_err(|_| invalid_config("invalid private payment verification key"))?,
            );
            let material = Zeroizing::new(raw.trim().as_bytes().to_vec());
            if provider == "epay" {
                others.push(material);
            } else {
                hmac_keys.push(material);
            }
        }
    }
    if let Some(path) = setting("LMM_CORE_WEB_REFRESH_KEY_FILE") {
        let key = Zeroizing::new(private_file(Path::new(&path), 128)?);
        hmac_keys.push(key);
    }
    if let Some(path) = setting("LMM_CORE_TELEGRAM_BOT_TOKEN_FILE") {
        // Match identity_store -> TelegramLogin::new exactly: decode UTF-8,
        // trim the configured token, then derive its 32-byte signing key.
        let token = Zeroizing::new(
            String::from_utf8(private_file(Path::new(&path), 4096)?)
                .map_err(|_| invalid_config("invalid Telegram credential"))?,
        );
        others.push(Zeroizing::new(
            Sha256::digest(token.trim().as_bytes()).to_vec(),
        ));
        others.push(Zeroizing::new(token.trim().as_bytes().to_vec()));
    }
    let mut keys = Vec::new();
    for key in ring.keys {
        let material = Zeroizing::new(read_managed(&key.key_file, 32)?);
        if others
            .iter()
            .any(|other| other.as_slice() == material.as_slice())
            || hmac_keys.iter().any(|key| hmac_sha256_uses(key, &material))
            || receipt_verifier
                .as_ref()
                .is_some_and(|verifier| verifier.uses_signing_key_material(&material))
        {
            return Err(invalid_config(
                "managed recovery keys must use independent key material",
            ));
        }
        keys.push((key.id, material));
    }
    let keys = keys
        .into_iter()
        .map(|(id, mut material)| (id, std::mem::take(&mut *material)))
        .collect();
    ManagedRecoveryKeys::new(&ring.active_key_id, keys)
        .map(Some)
        .map_err(|_| invalid_config("invalid managed recovery keys"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        fs,
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    };

    const MANAGED: &str = "LMM_CORE_OAUTH2_MANAGED_REFRESH_KEYRING_FILE";
    struct Fixture {
        root: PathBuf,
        secrets: PathBuf,
        settings: BTreeMap<&'static str, OsString>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "lmm-managed-keys-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            fs::create_dir(&root).unwrap();
            let root = fs::canonicalize(root).unwrap();
            let secrets = root.join("secrets");
            fs::create_dir(&secrets).unwrap();
            fs::set_permissions(&secrets, fs::Permissions::from_mode(0o700)).unwrap();
            let mut fixture = Self {
                root,
                secrets,
                settings: BTreeMap::new(),
            };
            fixture.key("current", 83);
            fixture.key("retired", 84);
            fixture.managed_ring(&fixture.secrets.join("current"));
            fixture
        }
        fn write(&self, path: &Path, bytes: &[u8]) -> PathBuf {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)
                .unwrap();
            file.write_all(bytes).unwrap();
            path.to_path_buf()
        }
        fn key(&self, name: &str, value: u8) -> PathBuf {
            self.write(&self.secrets.join(name), &[value; 32])
        }
        fn document(&self, name: &str, value: serde_json::Value) -> PathBuf {
            // Decode JSON numbers before TOML serialization: arbitrary_precision
            // otherwise emits serde_json's private numeric wrapper as a table.
            let value: toml::Table = serde_json::from_value(value).unwrap();
            self.write(
                &self.secrets.join(name),
                toml::to_string(&value).unwrap().as_bytes(),
            )
        }
        fn managed_ring(&mut self, current: &Path) {
            let path = self.document(
                "managed.toml",
                json!({
                    "version": 1, "active_key_id": "current", "keys": [
                        {"id": "current", "key_file": current},
                        {"id": "retired", "key_file": self.secrets.join("retired")}
                    ]
                }),
            );
            self.settings.insert(MANAGED, path.into_os_string());
        }
        fn load(&self, isolated: bool) -> io::Result<Option<ManagedRecoveryKeys>> {
            load(
                |name| self.settings.get(name).cloned(),
                isolated.then_some(self.secrets.as_path()),
            )
        }
        fn all_domains(&mut self) -> Vec<PathBuf> {
            let mut paths = Vec::new();
            for (name, setting) in [
                ("web", "LMM_CORE_WEB_REFRESH_KEY_FILE"),
                ("api", "LMM_CORE_API_KEY_ENCRYPTION_KEY_FILE"),
            ] {
                let key = self.key(name, 17 + paths.len() as u8);
                self.settings.insert(setting, key.clone().into_os_string());
                paths.push(key);
            }
            for (name, setting) in [
                ("mfa", "LMM_CORE_MFA_KEYRING_FILE"),
                ("federated", "LMM_CORE_FEDERATED_AUTH_KEYRING_FILE"),
            ] {
                let current = self.key(&format!("{name}-current"), 17 + paths.len() as u8);
                paths.push(current.clone());
                let retired = self.key(&format!("{name}-retired"), 17 + paths.len() as u8);
                paths.push(retired.clone());
                let mut ring = json!({"version": 1, "active_key_id": "current", "keys": [
                    {"id": "current", "key_file": current},
                    {"id": "retired", "key_file": retired}
                ]});
                if name == "federated" {
                    let replay = self.key("federated-replay", 17 + paths.len() as u8);
                    ring["replay_key_file"] = json!(replay);
                    paths.push(replay);
                }
                let path = self.document(&format!("{name}.toml"), ring);
                self.settings.insert(setting, path.into_os_string());
            }
            let proof = self.key("email-proof", 17 + paths.len() as u8);
            paths.push(proof.clone());
            let rate = self.key("email-rate", 17 + paths.len() as u8);
            paths.push(rate.clone());
            let current = self.key("email-current", 17 + paths.len() as u8);
            paths.push(current.clone());
            let retired = self.key("email-retired", 17 + paths.len() as u8);
            paths.push(retired.clone());
            let email = self.document("email.toml", json!({
                "challenge_key_id": "proof-v1", "proof_key_file": proof,
                "rate_key_file": rate, "active_key_id": "current",
                "key_files": {"current": current, "retired": retired},
                "reset_origin": "https://email.example.test", "smtp": {
                    "host": "smtp.example.test", "port": 587,
                    "from": "qa@example.test", "tls": "starttls",
                    "username": "fixture", "password_file": self.root.join("never-open-smtp-password")
                }
            }));
            self.settings
                .insert("LMM_CORE_EMAIL_CONFIG_FILE", email.into_os_string());
            paths
        }
    }

    #[test]
    fn managed_keys_reject_material_shared_with_every_configured_domain_and_retired_key() {
        let mut fixture = Fixture::new();
        let others = fixture.all_domains();
        assert_eq!(others.len(), 12);
        assert!(fixture.load(false).is_ok());
        assert!(fixture.load(true).is_ok());
        for (index, path) in others.iter().enumerate() {
            for material in [83, 84] {
                fixture.write(path, &[material; 32]);
                assert!(fixture.load(false).is_err(), "domain index {index}");
                assert!(fixture.load(true).is_err(), "isolated domain index {index}");
            }
            fixture.write(path, &[17 + index as u8; 32]);
            assert!(fixture.load(false).is_ok());
        }
        // Web refresh accepts longer raw keys; separation must not narrow that
        // existing format merely because managed AES keys are exactly 32 bytes.
        fixture.write(&others[0], &[41; 64]);
        assert!(fixture.load(false).is_ok());
        assert!(!fixture.root.join("never-open-smtp-password").exists());
    }

    #[test]
    fn managed_keys_reject_shared_material_through_hardlinks_and_parent_aliases() {
        let mut fixture = Fixture::new();
        let hardlink = fixture.secrets.join("web-hardlink");
        fs::hard_link(fixture.secrets.join("current"), &hardlink).unwrap();
        fixture.settings.insert(
            "LMM_CORE_WEB_REFRESH_KEY_FILE",
            hardlink.clone().into_os_string(),
        );
        assert!(fixture.load(false).is_err());
        fixture.settings.remove("LMM_CORE_WEB_REFRESH_KEY_FILE");
        fs::remove_file(hardlink).unwrap();
        let alias = fixture.root.join("parent-alias");
        symlink(&fixture.secrets, &alias).unwrap();
        fixture.settings.insert(
            "LMM_CORE_API_KEY_ENCRYPTION_KEY_FILE",
            alias.join("current").into_os_string(),
        );
        assert!(fixture.load(false).is_err());
        fixture
            .settings
            .remove("LMM_CORE_API_KEY_ENCRYPTION_KEY_FILE");
        fixture.write(&fixture.secrets.join("retired"), &[83; 32]);
        assert!(fixture.load(false).is_err());
        fixture.write(&fixture.secrets.join("retired"), &[84; 32]);
        assert!(fixture.load(false).is_ok());
    }

    #[test]
    fn managed_keys_reject_the_actual_trimmed_telegram_signing_key_including_retired_keys() {
        let mut fixture = Fixture::new();
        let token = fixture.write(
            &fixture.secrets.join("telegram-token"),
            b" \n4242:synthetic-qa-token\t\n",
        );
        fixture
            .settings
            .insert("LMM_CORE_TELEGRAM_BOT_TOKEN_FILE", token.into_os_string());
        let signing_key = Sha256::digest(b"4242:synthetic-qa-token");
        assert!(fixture.load(false).is_ok());
        for (name, original) in [("current", 83), ("retired", 84)] {
            fixture.write(&fixture.secrets.join(name), &signing_key);
            assert!(fixture.load(false).is_err());
            assert!(fixture.load(true).is_err());
            fixture.key(name, original);
        }
        assert!(fixture.load(false).is_ok());
    }

    #[test]
    fn managed_keys_reject_effective_web_hmac_material_without_narrowing_legal_key_lengths() {
        use hmac::{Hmac, KeyInit, Mac};
        let tag = |key: &[u8]| {
            let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
            mac.update(b"synthetic-browser-domain-and-session");
            mac.finalize().into_bytes()
        };
        let mut fixture = Fixture::new();
        let web = fixture.key("web", 41);
        fixture.settings.insert(
            "LMM_CORE_WEB_REFRESH_KEY_FILE",
            web.clone().into_os_string(),
        );
        let mut padded = vec![31; 32];
        padded.extend([0; 32]);
        for raw in [vec![41; 65], vec![41; 128], padded] {
            let effective = if raw.len() > 64 {
                Sha256::digest(&raw).to_vec()
            } else {
                raw[..32].to_vec()
            };
            // Prove equivalence using the same real HMAC primitive as BrowserAuth,
            // rather than assuming that two different raw lengths are distinct.
            assert_eq!(tag(&raw), tag(&effective));
            fixture.write(&web, &raw);
            assert!(fixture.load(false).is_ok());
            for (name, original) in [("current", 83), ("retired", 84)] {
                fixture.write(&fixture.secrets.join(name), &effective);
                assert!(fixture.load(false).is_err());
                fixture.key(name, original);
            }
        }
        // A nonzero HMAC suffix is not equivalent to its 32-byte prefix.
        let raw = [vec![31; 32], vec![41; 32]].concat();
        assert_ne!(tag(&raw), tag(&[31; 32]));
        fixture.write(&web, &raw);
        fixture.key("current", 31);
        assert!(fixture.load(false).is_ok());
    }

    #[test]
    fn isolated_managed_ring_and_each_key_must_be_independent_canonical_private_files_inside_registry()
     {
        let mut fixture = Fixture::new();
        assert!(fixture.load(true).is_ok());
        let ring = fixture.secrets.join("managed.toml");
        let outside = fixture.write(
            &fixture.root.join("outside-ring"),
            &fs::read(&ring).unwrap(),
        );
        fixture.settings.insert(MANAGED, outside.into_os_string());
        assert!(fixture.load(true).is_err());
        fixture
            .settings
            .insert(MANAGED, ring.clone().into_os_string());
        let linked_ring = fixture.secrets.join("linked-ring");
        symlink(&ring, &linked_ring).unwrap();
        fixture
            .settings
            .insert(MANAGED, linked_ring.into_os_string());
        assert!(fixture.load(false).is_err());
        assert!(fixture.load(true).is_err());
        fixture
            .settings
            .insert(MANAGED, ring.clone().into_os_string());
        let hard_ring = fixture.secrets.join("hard-ring");
        fs::hard_link(&ring, &hard_ring).unwrap();
        assert!(fixture.load(true).is_err());
        fs::remove_file(hard_ring).unwrap();

        let current = fixture.secrets.join("current");
        let outside_key = fixture.write(&fixture.root.join("outside-key"), &[83; 32]);
        let key_alias = fixture.secrets.join("key-alias");
        symlink(&current, &key_alias).unwrap();
        let directory_alias = fixture.secrets.join("directory-alias");
        symlink(&fixture.secrets, &directory_alias).unwrap();
        let sub = fixture.secrets.join("sub");
        fs::create_dir(&sub).unwrap();
        for path in [
            outside_key,
            key_alias,
            directory_alias.join("current"),
            sub.join("../current"),
        ] {
            fixture.managed_ring(&path);
            assert!(fixture.load(true).is_err());
        }
        fixture.managed_ring(&current);
        let hard_key = fixture.secrets.join("hard-key");
        fs::hard_link(&current, &hard_key).unwrap();
        assert!(fixture.load(true).is_err());
        fs::remove_file(hard_key).unwrap();
        assert!(fixture.load(true).is_ok());
        // Retired keys cannot escape the registry either.
        let retired = fixture.secrets.join("retired");
        fs::remove_file(&retired).unwrap();
        symlink(&current, &retired).unwrap();
        assert!(fixture.load(true).is_err());
        fs::remove_file(&retired).unwrap();
        fixture.key("retired", 84);
        fs::set_permissions(&fixture.secrets, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(fixture.load(true).is_err());
        fs::set_permissions(&fixture.secrets, fs::Permissions::from_mode(0o700)).unwrap();
        let alias = fixture.root.join("secret-directory-alias");
        symlink(&fixture.secrets, &alias).unwrap();
        assert!(load(|name| fixture.settings.get(name).cloned(), Some(&alias)).is_err());
        assert!(fixture.load(true).is_ok());
    }

    #[test]
    fn unconfigured_managed_or_other_uses_do_not_open_or_generate_keys() {
        assert!(
            load(
                |name| {
                    assert_eq!(
                        name, MANAGED,
                        "unconfigured managed keys must not scan other domains"
                    );
                    None
                },
                None
            )
            .unwrap()
            .is_none()
        );
        let mut fixture = Fixture::new();
        let files = || fs::read_dir(&fixture.secrets).unwrap().count();
        let before = files();
        assert!(fixture.load(false).unwrap().is_some());
        assert_eq!(files(), before);
        let missing = fixture.root.join("configured-but-missing-api-key");
        fixture.settings.insert(
            "LMM_CORE_API_KEY_ENCRYPTION_KEY_FILE",
            missing.clone().into_os_string(),
        );
        assert!(fixture.load(false).is_err());
        assert!(!missing.exists());
    }
}
