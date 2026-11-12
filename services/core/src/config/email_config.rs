//! Email sidecar configuration. Secrets and parser values never enter diagnostics.
use super::{
    absolute_path, bool_setting, configuration_file, invalid_config, parse_document, private_file,
    setting, setting_os,
};
use crate::{
    email_delivery::{MailKeys, Outbox, ResetSite, SmtpConfig, SmtpMailer, SmtpTls},
    email_http::EmailHttp,
    identity::{IdentityStore, email::EmailChallenges},
};
use serde::Deserialize;
use std::{collections::BTreeMap, env, io, path::PathBuf};
use zeroize::Zeroizing;

pub struct EmailRuntime {
    pub http: EmailHttp,
    pub outbox: Outbox,
    pub mailer: SmtpMailer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailFile {
    challenge_key_id: String,
    proof_key_file: PathBuf,
    rate_key_file: PathBuf,
    active_key_id: String,
    key_files: BTreeMap<String, PathBuf>,
    smtp: SmtpFile,
    reset_origin: String,
    #[serde(default)]
    local_http: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SmtpFile {
    host: String,
    port: u16,
    from: String,
    tls: TlsFile,
    username: Option<String>,
    password_file: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TlsFile {
    Starttls,
    Implicit,
    PlaintextLoopbackTest,
}

impl SmtpFile {
    fn into_config(self) -> SmtpConfig {
        SmtpConfig {
            host: self.host,
            port: self.port,
            from: self.from,
            tls: match self.tls {
                TlsFile::Starttls => SmtpTls::Starttls,
                TlsFile::Implicit => SmtpTls::Implicit,
                TlsFile::PlaintextLoopbackTest => SmtpTls::PlaintextLoopbackTest,
            },
            username: self.username,
            password_file: self.password_file,
        }
    }
}

pub(super) fn require_configuration(verification: bool, configured: bool) -> io::Result<()> {
    if verification && !configured {
        return Err(invalid_config(
            "email verification requires identity.email_config_file",
        ));
    }
    Ok(())
}

fn load_file(path: Option<PathBuf>, verification: bool) -> io::Result<Option<EmailFile>> {
    require_configuration(verification, path.is_some())?;
    path.map(|path| parse_document(&configuration_file(&path, 16 * 1024 * 1024)?))
        .transpose()
}

fn reset_site(origin: &str, local_http: bool, web: Option<(&str, bool)>) -> io::Result<ResetSite> {
    let parse = |origin, local| {
        if local {
            ResetSite::local_http(origin)
        } else {
            ResetSite::https(origin)
        }
        .map_err(|_| invalid_config("invalid email reset origin"))
    };
    let site = parse(origin, local_http)?;
    if let Some((web_origin, web_local_http)) = web {
        // Validate both complete origins before comparing their normalized URLs.
        // Comparing only origin() would silently discard paths and credentials.
        parse(web_origin, web_local_http)?;
        let normalize = |value| {
            reqwest::Url::parse(value).map_err(|_| invalid_config("invalid email reset origin"))
        };
        if local_http != web_local_http || normalize(origin)? != normalize(web_origin)? {
            return Err(invalid_config(
                "email reset origin and local_http must match Web configuration",
            ));
        }
    }
    Ok(site)
}

struct EmailKeys {
    proof: Zeroizing<[u8; 32]>,
    rate: Zeroizing<[u8; 32]>,
    mail: MailKeys,
}

fn read_key(path: &std::path::Path) -> io::Result<Zeroizing<[u8; 32]>> {
    let bytes = Zeroizing::new(private_file(path, 32)?);
    if bytes.len() != 32 || bytes.iter().all(|b| *b == 0) {
        return Err(invalid_config(
            "email keys require nonzero raw 32-byte files",
        ));
    }
    let mut key = Zeroizing::new([0; 32]);
    key.copy_from_slice(&bytes);
    Ok(key)
}

impl EmailFile {
    fn load_keys(&self) -> io::Result<EmailKeys> {
        let proof = read_key(&self.proof_key_file)?;
        let rate = read_key(&self.rate_key_file)?;
        if *proof == *rate {
            return Err(invalid_config(
                "email proof and rate keys must be independent",
            ));
        }
        let files: Vec<_> = self
            .key_files
            .iter()
            .map(|(id, path)| (id.clone(), path.clone()))
            .collect();
        for (_, path) in &files {
            let key = read_key(path)?;
            if *key == *proof || *key == *rate {
                return Err(invalid_config(
                    "email encryption keys must be independent from proof and rate keys",
                ));
            }
        }
        let mail = MailKeys::from_files(&self.active_key_id, &files)
            .map_err(|_| invalid_config("invalid email encryption key configuration"))?;
        Ok(EmailKeys { proof, rate, mail })
    }
}

/// Constructs the email host without connecting to PostgreSQL or SMTP.
/// The executable owns starting and stopping the outbox worker.
pub fn email_runtime(store: IdentityStore, pool: sqlx::PgPool) -> io::Result<Option<EmailRuntime>> {
    let Some(file) = load_file(
        setting_os("LMM_CORE_EMAIL_CONFIG_FILE").map(PathBuf::from),
        bool_setting("LMM_CORE_EMAIL_VERIFICATION_ENABLED")?,
    )?
    else {
        return Ok(None);
    };
    let web_origin = match setting("LMM_CORE_WEB_ORIGIN") {
        Ok(origin) => Some(origin),
        Err(env::VarError::NotPresent) => None,
        Err(_) => return Err(invalid_config("invalid Web origin")),
    };
    let web = web_origin
        .as_deref()
        .map(|origin| -> io::Result<_> { Ok((origin, bool_setting("LMM_CORE_WEB_LOCAL_HTTP")?)) })
        .transpose()?;
    let reset = reset_site(&file.reset_origin, file.local_http, web)?;
    let keys = file.load_keys()?;
    let challenges = EmailChallenges::new(
        pool.clone(),
        &file.challenge_key_id,
        *keys.proof,
        *keys.rate,
    )
    .map_err(|_| invalid_config("invalid email challenge key configuration"))?;
    let smtp = file.smtp.into_config();
    if let Some(path) = &smtp.password_file {
        absolute_path(path)?;
        // Apply the same core secret-file boundary; SmtpMailer validates content.
        let _secret = Zeroizing::new(private_file(path, 4096)?);
    }
    let mailer = SmtpMailer::open(&smtp, reset)
        .map_err(|_| invalid_config("invalid email SMTP configuration"))?;
    let outbox = Outbox::new(pool, keys.mail);
    let http = EmailHttp::new(store, challenges, outbox.clone());
    Ok(Some(EmailRuntime {
        http,
        outbox,
        mailer,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = r#"
challenge_key_id = "proof-v1"
proof_key_file = "/nonexistent-email-test/proof"
rate_key_file = "/nonexistent-email-test/rate"
active_key_id = "mail-v1"
reset_origin = "https://app.example.test"
[key_files]
mail-v1 = "/nonexistent-email-test/mail"
[smtp]
host = "smtp.example.test"
port = 587
from = "noreply@example.test"
tls = "starttls"
username = "smtp-user"
password_file = "/nonexistent-email-test/smtp-password"
"#;

    #[test]
    fn sidecar_requires_separate_keys_and_explicit_smtp_transport() {
        let file: EmailFile = parse_document(DOCUMENT.as_bytes()).unwrap();
        assert!(!file.local_http);
        assert!(matches!(file.smtp.into_config().tls, SmtpTls::Starttls));
        for name in [
            "challenge_key_id",
            "proof_key_file",
            "rate_key_file",
            "active_key_id",
            "reset_origin",
            "host",
            "port",
            "from",
            "tls",
        ] {
            let missing = DOCUMENT
                .lines()
                .filter(|line| !line.starts_with(name))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                parse_document::<EmailFile>(missing.as_bytes()).is_err(),
                "{name}"
            );
        }
        for invalid in [
            DOCUMENT.replace("tls = \"starttls\"", "tls = \"none\""),
            DOCUMENT.replace(
                "[key_files]\nmail-v1 = \"/nonexistent-email-test/mail\"\n",
                "",
            ),
            format!("password = 'do-not-echo-secret'\n{DOCUMENT}"),
            format!("{DOCUMENT}\npassword = 'do-not-echo-secret'"),
        ] {
            let error = parse_document::<EmailFile>(invalid.as_bytes())
                .err()
                .unwrap();
            assert!(!error.to_string().contains("do-not-echo-secret"));
        }
    }

    #[test]
    fn missing_mail_config_cannot_advertise_verification() {
        assert!(load_file(None, false).unwrap().is_none());
        assert!(load_file(None, true).is_err());
        let error = super::super::parse_core_file(
            b"version=1\n[identity]\nemail_verification_enabled=true",
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "email verification requires identity.email_config_file"
        );
        let document = b"version=1\n[database]\nurl_file='/nonexistent-email-test/db'\n[identity]\nemail_config_file='/nonexistent-email-test/email.toml'\nemail_verification_enabled=true";
        let values = super::super::parse_core_file(document).unwrap();
        assert_eq!(
            values["LMM_CORE_EMAIL_CONFIG_FILE"],
            "/nonexistent-email-test/email.toml"
        );
        let relative = String::from_utf8(document.to_vec())
            .unwrap()
            .replace("/nonexistent-email-test/email.toml", "email.toml");
        assert!(super::super::parse_core_file(relative.as_bytes()).is_err());
        assert!(
            super::super::parse_core_file(
                b"version=1\n[identity]\nemail_config_file='/nonexistent-email-test/email.toml'"
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn legacy_email_startup_uses_selected_source_and_fails_closed() {
        const CASE: &str = "LMM_TEST_EMAIL_RUNTIME_CONFIG_CASE";
        if let Ok(case) = env::var(CASE) {
            super::super::initialize(None, true).unwrap();
            let required = super::super::identity_database_required();
            let pool = sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://test:test@127.0.0.1:1/unused")
                .unwrap();
            let runtime = email_runtime(IdentityStore::from_pool(pool.clone()), pool);
            match case.as_str() {
                "disabled" => {
                    assert!(!required.unwrap());
                    assert!(runtime.unwrap().is_none());
                }
                "missing" => {
                    assert!(required.is_err());
                    assert!(runtime.is_err());
                }
                "configured" => {
                    assert!(required.unwrap());
                    // A selected but unavailable sidecar cannot silently disable email.
                    assert!(runtime.is_err());
                }
                _ => panic!("unknown test case"),
            }
            return;
        }
        for case in ["disabled", "missing", "configured"] {
            let mut command = std::process::Command::new(env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "config::email_config::tests::legacy_email_startup_uses_selected_source_and_fails_closed",
                    "--nocapture",
                ])
                .env_clear()
                .env(CASE, case)
                .env("LMM_CORE_EMAIL_VERIFICATION_ENABLED", if case == "missing" {
                    "true"
                } else {
                    "false"
                });
            if case == "configured" {
                command.env(
                    "LMM_CORE_EMAIL_CONFIG_FILE",
                    "/nonexistent-email-test/email.toml",
                );
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{case}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn reset_origin_is_normalized_and_bound_to_explicit_web_profile() {
        assert!(
            reset_site(
                "https://APP.example.test:443/",
                false,
                Some(("https://app.example.test", false))
            )
            .is_ok()
        );
        assert!(
            reset_site(
                "http://api.lmm.local:8080",
                true,
                Some(("http://api.lmm.local:8080/", true))
            )
            .is_ok()
        );
        for origin in [
            "http://localhost:8080",
            "https://user@app.example.test",
            "https://app.example.test/reset",
            "https://app.example.test?x=1",
            "https://app.example.test#token",
            " https://app.example.test",
        ] {
            assert!(reset_site(origin, false, None).is_err(), "{origin}");
        }
        assert!(reset_site("http://external.example.test", true, None).is_err());
        assert!(reset_site("https://app.example.test", true, None).is_err());
        for web in [
            ("https://other.example.test", false),
            ("https://app.example.test:8443", false),
            ("https://app.example.test", true),
            ("https://app.example.test/path", false),
        ] {
            assert!(reset_site("https://app.example.test", false, Some(web)).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn key_files_reject_cross_domain_reuse_bad_material_and_unsafe_files() {
        use std::{
            fs,
            io::Write,
            os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
        };
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let dir = std::env::temp_dir().join(format!(
            "lmm-email-config-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&dir).unwrap();
        let fixture = Fixture(fs::canonicalize(dir).unwrap());
        let write = |name: &str, bytes: &[u8]| {
            let path = fixture.0.join(name);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)
                .unwrap();
            file.write_all(bytes).unwrap();
            path
        };
        let mut file: EmailFile = parse_document(DOCUMENT.as_bytes()).unwrap();
        file.proof_key_file = write("proof", &[1; 32]);
        file.rate_key_file = write("rate", &[2; 32]);
        file.key_files = BTreeMap::from([
            ("mail-v1".into(), write("active", &[3; 32])),
            ("mail-old".into(), write("old", &[4; 32])),
        ]);
        assert!(file.load_keys().is_ok());
        file.active_key_id = "missing".into();
        assert!(file.load_keys().is_err());
        file.active_key_id = "mail-v1".into();
        write("rate", &[1; 32]);
        assert!(file.load_keys().is_err());
        write("rate", &[2; 32]);
        for name in ["active", "old"] {
            for shared in [1, 2] {
                write(name, &[shared; 32]);
                assert!(file.load_keys().is_err());
            }
            write(name, &[3; 32]);
        }
        for bytes in [vec![0; 32], vec![1; 31], vec![1; 33], vec![b'a'; 64]] {
            write("proof", &bytes);
            assert!(file.load_keys().is_err());
        }
        write("proof", &[1; 32]);
        fs::set_permissions(&file.proof_key_file, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(file.load_keys().is_err());
        fs::set_permissions(&file.proof_key_file, fs::Permissions::from_mode(0o600)).unwrap();
        let link = fixture.0.join("proof-link");
        symlink(&file.proof_key_file, &link).unwrap();
        file.proof_key_file = link;
        assert!(file.load_keys().is_err());
    }
}
