//! Explicit independent policy-editor authority; never inferred from root identity.
use super::*;
use crate::options_http::trust_policy::{EditorConfig, Service, SigningKey};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EditorFile {
    #[serde(default)]
    enabled: bool,
    rp_id: Option<String>,
    origin: Option<String>,
    signing_key_file: Option<PathBuf>,
}

pub(super) fn validate(
    editor: Option<&EditorFile>,
    console: bool,
    web_origin: Option<&str>,
    database: bool,
) -> io::Result<()> {
    let Some(editor) = editor else {
        return Ok(());
    };
    if !database {
        return Err(invalid_config(
            "trust policy editor requires its explicit database authority",
        ));
    }
    match (&editor.rp_id, &editor.origin) {
        (Some(rp), Some(origin)) => {
            EditorConfig::new(rp, origin)
                .map_err(|_| invalid_config("invalid trust policy editor WebAuthn origin"))?;
        }
        (None, None) if !editor.enabled && editor.signing_key_file.is_none() => {}
        _ => {
            return Err(invalid_config(
                "trust policy editor requires both RP ID and origin",
            ));
        }
    }
    if editor.enabled
        && (!console
            || editor.signing_key_file.is_none()
            || web_origin.map(|v| v.trim_end_matches('/'))
                != editor.origin.as_deref().map(|v| v.trim_end_matches('/')))
    {
        return Err(invalid_config(
            "online trust policy editor requires console, matching native Web origin and dedicated signing key",
        ));
    }
    Ok(())
}

pub(super) fn export(
    editor: EditorFile,
    put: &mut impl FnMut(&'static str, String),
    paths: &mut Vec<(&'static str, PathBuf)>,
) {
    put(
        "LMM_CORE_TRUST_POLICY_EDITOR_ENABLED",
        editor.enabled.to_string(),
    );
    if let Some(rp) = editor.rp_id {
        put("LMM_CORE_TRUST_POLICY_EDITOR_RP_ID", rp);
    }
    if let Some(origin) = editor.origin {
        put("LMM_CORE_TRUST_POLICY_EDITOR_ORIGIN", origin);
    }
    if let Some(path) = editor.signing_key_file {
        paths.push(("LMM_CORE_TRUST_POLICY_EDITOR_SIGNING_KEY_FILE", path));
    }
}

pub fn trust_policy_editor_enabled() -> io::Result<bool> {
    bool_setting("LMM_CORE_TRUST_POLICY_EDITOR_ENABLED")
}

/// Also used by the explicit offline owner CLI. It does not load a database URL.
pub fn trust_policy_editor_config() -> io::Result<EditorConfig> {
    let rp = setting("LMM_CORE_TRUST_POLICY_EDITOR_RP_ID")
        .map_err(|_| invalid_config("trust policy editor RP ID required"))?;
    let origin = setting("LMM_CORE_TRUST_POLICY_EDITOR_ORIGIN")
        .map_err(|_| invalid_config("trust policy editor origin required"))?;
    EditorConfig::new(&rp, &origin)
        .map_err(|_| invalid_config("invalid trust policy editor WebAuthn origin"))
}

fn signing_key_file(path: &Path) -> io::Result<Zeroizing<[u8; 32]>> {
    let (bytes, metadata) = read_file_with_metadata(path, 32, true)?;
    let bytes = Zeroizing::new(bytes);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        // Kernel-owned proc data avoids a new libc dependency or trusting UID env.
        let status = fs::read_to_string("/proc/self/status")
            .map_err(|_| invalid_config("cannot verify signing key owner"))?;
        let uid = status
            .lines()
            .find_map(|line| {
                line.strip_prefix("Uid:")?
                    .split_whitespace()
                    .nth(1)?
                    .parse::<u32>()
                    .ok()
            })
            .ok_or_else(|| invalid_config("cannot verify signing key owner"))?;
        if metadata.uid() != uid && metadata.uid() != 0 {
            return Err(invalid_config(
                "signing key must belong to the process user or root",
            ));
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = metadata;
        return Err(invalid_config(
            "policy editor key ownership verification requires Linux",
        ));
    }
    let mut key = Zeroizing::new([0_u8; 32]);
    if bytes.len() != key.len() {
        return Err(invalid_config(
            "policy signing key must contain exactly 32 random bytes",
        ));
    }
    key.copy_from_slice(&bytes);
    Ok(key)
}

pub fn trust_policy_signing_key() -> io::Result<Zeroizing<[u8; 32]>> {
    let path = setting_os("LMM_CORE_TRUST_POLICY_EDITOR_SIGNING_KEY_FILE")
        .ok_or_else(|| invalid_config("dedicated policy signing key file required"))?;
    let key = signing_key_file(Path::new(&path))?;
    let mut others: Vec<PathBuf> = [
        "LMM_CORE_API_KEY_ENCRYPTION_KEY_FILE",
        "LMM_CORE_WEB_REFRESH_KEY_FILE",
    ]
    .into_iter()
    .filter_map(setting_os)
    .map(PathBuf::from)
    .collect();
    if let Some(path) = setting_os("LMM_CORE_MFA_KEYRING_FILE") {
        let ring: MfaKeyringFile = parse_document(&private_file(Path::new(&path), 16 * 1024)?)?;
        others.extend(ring.keys.into_iter().map(|entry| entry.key_file));
    }
    for other in others {
        let bytes = Zeroizing::new(private_file(&other, 16 * 1024)?);
        if bytes.as_slice() == key.as_slice() {
            return Err(invalid_config(
                "policy signing key cannot reuse identity or session key material",
            ));
        }
    }
    Ok(key)
}

pub async fn trust_policy_editor(pool: sqlx::PgPool) -> io::Result<Option<Service>> {
    if !trust_policy_editor_enabled()? {
        return Ok(None);
    }
    let origin = setting("LMM_CORE_TRUST_POLICY_EDITOR_ORIGIN")
        .map_err(|_| invalid_config("trust policy editor origin required"))?;
    let web = setting("LMM_CORE_WEB_ORIGIN")
        .map_err(|_| invalid_config("trust policy editor requires native Web identity"))?;
    if !console_enabled()? || origin.trim_end_matches('/') != web.trim_end_matches('/') {
        return Err(invalid_config(
            "trust policy editor origin must match the enabled native console",
        ));
    }
    let key = trust_policy_signing_key()?;
    let service = Service::new(pool, trust_policy_editor_config()?, SigningKey::new(*key));
    service
        .check_ready()
        .await
        .map_err(|_| invalid_config("trust policy editor authority or signing key unavailable"))?;
    Ok(Some(service))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn online_editor_requires_independent_complete_configuration() {
        let prefix = "version=1\n[database]\nurl_file='/isolated-test/database'\n[console]\nenabled=true\n[identity.web]\norigin='https://api.example.test'\nrefresh_key_file='/isolated-test/refresh'\nuser_agreement_enabled=false\nprivacy_policy_enabled=false\n";
        let editor = "[trust_policy_editor]\nenabled=true\nrp_id='api.example.test'\norigin='https://api.example.test'\nsigning_key_file='/isolated-test/editor-key'\n";
        let parsed = parse_core_file(format!("{prefix}{editor}").as_bytes()).unwrap();
        assert_eq!(parsed["LMM_CORE_TRUST_POLICY_EDITOR_ENABLED"], "true");
        for bad in [
            editor.replace("signing_key_file='/isolated-test/editor-key'\n", ""),
            editor.replace("https://api.example.test", "https://other.example.test"),
            editor.replace("rp_id='api.example.test'\n", ""),
            editor.replace(
                "signing_key_file='/isolated-test/editor-key'",
                "signing_key_file='relative-key'",
            ),
        ] {
            assert!(parse_core_file(format!("{prefix}{bad}").as_bytes()).is_err());
        }
        assert!(parse_core_file(format!("version=1\n{editor}").as_bytes()).is_err());
        assert!(
            parse_core_file(
                format!("version=1\n[database]\nurl_file='/isolated-test/database'\n{editor}")
                    .as_bytes()
            )
            .is_err()
        );
    }
}
