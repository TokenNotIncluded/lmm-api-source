//! Select an explicit sealed Core for process tests, or Cargo's ordinary binary.
//! The two environment values are one input: incomplete or invalid pairs fail.
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
};

pub fn path() -> PathBuf {
    match (
        std::env::var_os("LMM_TEST_CORE_BIN"),
        std::env::var_os("LMM_TEST_CORE_SHA256"),
    ) {
        (None, None) => PathBuf::from(env!("CARGO_BIN_EXE_lmm-core")),
        (Some(path), Some(expected)) => {
            let path = PathBuf::from(path);
            assert!(path.is_absolute(), "LMM_TEST_CORE_BIN must be absolute");
            let metadata = fs::symlink_metadata(&path)
                .expect("LMM_TEST_CORE_BIN must be an existing regular file");
            assert!(
                metadata.file_type().is_file(),
                "LMM_TEST_CORE_BIN must be a regular file, not a symlink"
            );
            assert_eq!(
                fs::canonicalize(&path).expect("LMM_TEST_CORE_BIN must resolve"),
                path,
                "LMM_TEST_CORE_BIN must not contain symlink or indirect path components"
            );
            let expected = expected
                .to_str()
                .expect("LMM_TEST_CORE_SHA256 must be UTF-8");
            assert!(
                expected.len() == 64
                    && expected
                        .bytes()
                        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
                "LMM_TEST_CORE_SHA256 must be 64 lowercase hexadecimal characters"
            );
            let mut file = File::open(&path).expect("LMM_TEST_CORE_BIN must be readable");
            let mut digest = Sha256::new();
            let mut buffer = [0_u8; 8192];
            loop {
                let count = file
                    .read(&mut buffer)
                    .expect("LMM_TEST_CORE_BIN must remain readable");
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
            let mut actual = [0_u8; 64];
            zhex::encode_lower(digest.finalize().as_ref(), &mut actual)
                .expect("SHA-256 has exact hexadecimal output capacity");
            assert_eq!(
                actual.as_slice(),
                expected.as_bytes(),
                "LMM_TEST_CORE_SHA256 does not match LMM_TEST_CORE_BIN"
            );
            path
        }
        _ => panic!("LMM_TEST_CORE_BIN and LMM_TEST_CORE_SHA256 must be set together"),
    }
}
