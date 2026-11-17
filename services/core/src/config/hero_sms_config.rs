// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HeroSmsFile {
    pub socket: PathBuf,
    pub token_file: PathBuf,
    pub policy_file: PathBuf,
    pub api_key_file: Option<PathBuf>,
    pub shared_group_id: Option<u32>,
}

pub fn hero_sms_configured() -> bool {
    setting_os("LMM_CORE_HERO_SMS_TOKEN_FILE").is_some()
}

pub(super) fn service(pool: sqlx::PgPool) -> io::Result<crate::hero_sms::Service> {
    let path = setting_os("LMM_CORE_HERO_SMS_POLICY_FILE")
        .ok_or_else(|| invalid_config("HeroSMS requires a private policy file"))?;
    let policy = crate::hero_sms::PolicyFile::open(PathBuf::from(path))
        .map_err(|_| invalid_config("invalid HeroSMS policy file"))?;
    let credential = setting_os("LMM_CORE_HERO_SMS_API_KEY_FILE").map(PathBuf::from);
    crate::hero_sms::Service::new(pool, policy, credential)
        .map_err(|_| invalid_config("invalid HeroSMS supplier configuration"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_requires_database_and_separate_private_files() {
        let section = r#"
[hero_sms]
socket="/run/lmm/core/private.sock"
token_file="/private/hero-token"
policy_file="/private/hero-policy.json"
api_key_file="/private/hero-key"
shared_group_id=12345
"#;
        assert!(parse_core_file(format!("version=1\n{section}").as_bytes()).is_err());
        let document = format!("version=1\n[database]\nurl_file='/private/db'\n{section}");
        let values = parse_core_file(document.as_bytes()).unwrap();
        assert_eq!(
            values["LMM_CORE_HERO_SMS_POLICY_FILE"],
            "/private/hero-policy.json"
        );
        assert_eq!(values["LMM_CORE_HERO_SMS_SHARED_GROUP_ID"], "12345");
        for invalid in [
            document.replace("/private/hero-key", "/private/hero-token"),
            document.replace("/private/hero-key", "/private/db"),
            document.replace("/private/hero-policy.json", "/private/db"),
            document.replace("shared_group_id=12345", "shared_group_id=0"),
            document.replace("/private/hero-policy.json", "relative.json"),
        ] {
            assert!(parse_core_file(invalid.as_bytes()).is_err());
        }
    }
}
