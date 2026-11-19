//! Optional production-native library inputs for the unchanged commerce workload.
use super::{Process, observation, private};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declaration {
    format: String,
    host_library: PathBuf,
    host_sha256: String,
    plugin_library: PathBuf,
    plugin_sha256: String,
}

pub struct Inputs {
    declaration: Declaration,
    declaration_sha256: String,
    key: PathBuf,
}

fn verified(path: &Path, expected: &str) -> fs::Metadata {
    assert!(path.is_absolute() && path.is_file());
    assert_eq!(fs::canonicalize(path).unwrap(), path);
    assert!(
        expected.len() == 64
            && expected
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    let mut file = fs::File::open(path).unwrap();
    let before = file.metadata().unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(
        hex_codec::encode(Sha256::digest(&bytes)),
        expected,
        "native library differs from its frozen official input"
    );
    let after = fs::metadata(path).unwrap();
    assert_eq!(
        (before.dev(), before.ino(), before.len()),
        (after.dev(), after.ino(), after.len())
    );
    before
}

impl Inputs {
    pub fn from_environment(root: &Path) -> Option<Self> {
        let Some(path) = std::env::var_os("LMM_COMMERCE_EXTORE_NATIVE_INPUT") else {
            assert!(std::env::var_os("LMM_COMMERCE_EXTORE_NATIVE_SHA256").is_none());
            return None;
        };
        let path = PathBuf::from(path);
        let expected = std::env::var("LMM_COMMERCE_EXTORE_NATIVE_SHA256")
            .expect("native input requires its frozen SHA256");
        verified(&path, &expected);
        let declaration: Declaration = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(declaration.format, "lmm-native-extore-input-v1");
        verified(&declaration.host_library, &declaration.host_sha256);
        verified(&declaration.plugin_library, &declaration.plugin_sha256);
        assert_ne!(declaration.host_library, declaration.plugin_library);
        let mut key_bytes = [0_u8; 32];
        fs::File::open("/dev/urandom")
            .unwrap()
            .read_exact(&mut key_bytes)
            .unwrap();
        let key = private(root, "extore-native.key", &key_bytes);
        Some(Self {
            declaration,
            declaration_sha256: expected,
            key,
        })
    }

    pub fn store_settings(&self) -> String {
        let quoted = |path: &Path| serde_json::to_string(path.to_str().unwrap()).unwrap();
        format!(
            "extore_key_file={}\nextore_redirect_uri='https://api.lmm.best/store/manage'\n[store.extore_plugin]\nhost_library={}\nhost_sha256='{}'\nplugin_library={}\nplugin_sha256='{}'\n",
            quoted(&self.key),
            quoted(&self.declaration.host_library),
            self.declaration.host_sha256,
            quoted(&self.declaration.plugin_library),
            self.declaration.plugin_sha256
        )
    }

    pub fn observe(&self, process: &Process, stage: &str) {
        let pid = process.0.id();
        let identity = observation(pid, process.1["executed_sha256"].as_str().unwrap());
        let maps = fs::read_to_string(format!("/proc/{pid}/maps")).unwrap();
        let mut libraries = Vec::new();
        for (kind, path, expected) in [
            (
                "host",
                &self.declaration.host_library,
                &self.declaration.host_sha256,
            ),
            (
                "plugin",
                &self.declaration.plugin_library,
                &self.declaration.plugin_sha256,
            ),
        ] {
            let metadata = verified(path, expected);
            let matching: Vec<&str> = maps
                .lines()
                .filter(|line| {
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    fields.len() >= 6
                        && fields[4].parse::<u64>().ok() == Some(metadata.ino())
                        && fields[5..].join(" ") == path.to_str().unwrap()
                })
                .collect();
            assert!(
                !matching.is_empty(),
                "frozen native {kind} library is not mapped by the actual Go PID"
            );
            assert!(
                matching
                    .iter()
                    .any(|line| line.split_whitespace().nth(1).unwrap().contains('x')),
                "native image has no executable mapping"
            );
            libraries.push(json!({"kind":kind,"path":path,"sha256":expected,"device":metadata.dev(),"inode":metadata.ino(),"maps":matching}));
        }
        assert_eq!(
            identity,
            observation(pid, process.1["executed_sha256"].as_str().unwrap())
        );
        private(
            &process.2,
            &format!("native-libraries.{stage}.json"),
            &serde_json::to_vec(&json!({
                "format":"lmm-native-library-observation-v1","stage":stage,"pid":pid,
                "identity":identity,"native_input_sha256":self.declaration_sha256,
                "libraries":libraries,"public_origin":"https://api.lmm.best",
                "redirect_uri":"https://api.lmm.best/store/manage",
                "scope":"Startup activation; this harness does not issue an Extore catalog request"
            }))
            .unwrap(),
        );
    }
}
