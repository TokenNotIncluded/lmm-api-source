//! Bounded, read-only host and native-process resource sampling.
//!
//! This module deliberately reports only aggregate counters. It never exposes
//! command lines, environment variables, file paths, credentials, or arbitrary
//! PIDs supplied by a caller.

use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

const SAMPLE_WINDOW_MS: u64 = 250;
const CORE_PROCESS: &str = "lmm-core";
const EXTENSIONS_PROCESS: &str = "lmm-extensions";

#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub observed_at_unix_ms: i64,
    pub sample_window_ms: u64,
    pub system: SystemResources,
    pub processes: BTreeMap<String, ProcessResources>,
}

#[derive(Debug, Serialize)]
pub struct SystemResources {
    pub logical_cpus: u32,
    pub load_average: LoadAverage,
    pub cpu_usage_percent: f64,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub memory_used_bytes: u64,
    pub memory_usage_percent: f64,
}

#[derive(Debug, Serialize)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

#[derive(Debug, Serialize)]
pub struct ProcessResources {
    pub status: &'static str,
    pub version: Option<String>,
    pub pid: Option<u32>,
    pub cpu_percent: Option<f64>,
    pub memory_bytes: Option<u64>,
    pub memory_percent: Option<f64>,
    pub threads: Option<u64>,
}

#[derive(Clone, Copy)]
struct ProcessSample {
    pid: u32,
    start_time: u64,
    cpu_usage: f32,
    rss_bytes: u64,
    threads: Option<u64>,
}

#[derive(Debug)]
pub enum SnapshotError {
    Io,
    Invalid,
}

impl From<io::Error> for SnapshotError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

pub async fn snapshot() -> Result<Snapshot, SnapshotError> {
    let mut host = System::new().map_err(|_| SnapshotError::Io)?;
    host.refresh_cpu_usage();
    let before = process_samples(&mut host);
    tokio::time::sleep(std::time::Duration::from_millis(SAMPLE_WINDOW_MS)).await;
    host.refresh_cpu_usage();
    host.refresh_memory();
    let after = process_samples(&mut host);
    let logical_cpus = host.cpus().len();
    let total_bytes = host.total_memory();
    if logical_cpus == 0 || total_bytes == 0 {
        return Err(SnapshotError::Invalid);
    }
    let available_bytes = host.available_memory();
    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let cpu_usage_percent = f64::from(host.global_cpu_usage()).clamp(0.0, 100.0);
    let load = System::load_average().map_err(|_| SnapshotError::Io)?;
    let mut processes = BTreeMap::new();
    for name in [CORE_PROCESS, EXTENSIONS_PROCESS] {
        let version = process_version(name);
        let sample = after.get(name).copied();
        let previous = before.get(name).copied();
        let cpu_percent = match (sample, previous) {
            (Some(now), Some(old)) if now.pid == old.pid && now.start_time == old.start_time => {
                Some(normalized_cpu_percent(now.cpu_usage, logical_cpus))
            }
            _ => None,
        };
        let process = sample.map(|sample| ProcessResources {
            status: "running",
            version: version.clone(),
            pid: Some(sample.pid),
            cpu_percent,
            memory_bytes: Some(sample.rss_bytes),
            memory_percent: Some(percent(sample.rss_bytes, total_bytes)),
            threads: sample.threads,
        });
        processes.insert(
            name.to_owned(),
            process.unwrap_or(ProcessResources {
                status: "unavailable",
                version,
                pid: None,
                cpu_percent: None,
                memory_bytes: None,
                memory_percent: None,
                threads: None,
            }),
        );
    }
    Ok(Snapshot {
        schema_version: 1,
        observed_at_unix_ms: now_unix_ms(),
        sample_window_ms: SAMPLE_WINDOW_MS,
        system: SystemResources {
            logical_cpus: logical_cpus.min(u32::MAX as usize) as u32,
            load_average: LoadAverage {
                one: load.one,
                five: load.five,
                fifteen: load.fifteen,
            },
            cpu_usage_percent,
            memory_total_bytes: total_bytes,
            memory_available_bytes: available_bytes,
            memory_used_bytes: used_bytes,
            memory_usage_percent: percent(used_bytes, total_bytes),
        },
        processes,
    })
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_millis()).ok())
        .unwrap_or(0)
}

fn percent(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    ((numerator as f64 / denominator as f64) * 100.0).clamp(0.0, 100.0)
}

fn normalized_cpu_percent(usage: f32, logical_cpus: usize) -> f64 {
    if logical_cpus == 0 || !usage.is_finite() {
        return 0.0;
    }
    (f64::from(usage) / logical_cpus as f64).clamp(0.0, 100.0)
}

fn process_samples(host: &mut System) -> BTreeMap<&'static str, ProcessSample> {
    // Refresh only resource counters and executable identity; never collect command
    // lines, environment, working directories or user credentials.
    host.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_cpu()
            .with_memory()
            .with_exe(UpdateKind::Always),
    );
    let mut result = BTreeMap::new();
    for name in [CORE_PROCESS, EXTENSIONS_PROCESS] {
        let pid = if name == CORE_PROCESS {
            Some(Pid::from_u32(std::process::id()))
        } else {
            host.processes().iter().find_map(|(pid, process)| {
                // Keep the permission-aware identity fallback: sysinfo's absent
                // executable alone cannot distinguish denial from a dead process.
                let executable = process.exe().map_or_else(
                    || fs::read_link(format!("/proc/{pid}/exe")),
                    |path| Ok(path.to_owned()),
                );
                process_name_matches(name, executable, || {
                    Ok(process.name().to_string_lossy().into_owned())
                })
                .then_some(*pid)
            })
        };
        let Some(pid) = pid else { continue };
        // Only enumerate tasks for the two selected services. Linux's task set
        // excludes the process leader, which also counts as one thread.
        host.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_tasks(),
        );
        if let Some(process) = host.process(pid) {
            result.insert(
                name,
                ProcessSample {
                    pid: pid.as_u32(),
                    start_time: process.start_time(),
                    cpu_usage: process.cpu_usage(),
                    rss_bytes: process.memory(),
                    threads: process.tasks().map(|tasks| tasks.len() as u64 + 1),
                },
            );
        }
    }
    result
}

// Cross-user /proc/exe reads require ptrace permission; comm and resource counters
// remain readable. This name match is for aggregate diagnostics, not authorization.
fn process_name_matches(
    name: &str,
    executable: io::Result<PathBuf>,
    read_comm: impl FnOnce() -> io::Result<String>,
) -> bool {
    // @context-fold begin native-process-permission-aware-name-match
    match executable {
        Ok(path) => path.file_name().is_some_and(|value| value == name),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            read_comm().is_ok_and(|comm| comm.strip_suffix('\n').unwrap_or(&comm) == name)
        }
        Err(_) => false,
    }
    // @context-fold end native-process-permission-aware-name-match
}

fn process_version(name: &str) -> Option<String> {
    if name == CORE_PROCESS {
        return Some(env!("CARGO_PKG_VERSION").to_owned());
    }
    let bytes = fs::read("/usr/share/doc/lmm-extensions/BUILDINFO.json").ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value.get("version")?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{EXTENSIONS_PROCESS, normalized_cpu_percent, percent, process_name_matches};
    use std::{io, path::PathBuf};

    #[test]
    fn percent_is_bounded_and_zero_safe() {
        assert_eq!(percent(0, 0), 0.0);
        assert_eq!(percent(2, 1), 100.0);
        assert_eq!(percent(1, 4), 25.0);
    }

    #[test]
    fn process_cpu_is_normalized_to_whole_machine_capacity() {
        assert_eq!(normalized_cpu_percent(100.0, 4), 25.0);
        assert_eq!(normalized_cpu_percent(400.0, 4), 100.0);
        assert_eq!(normalized_cpu_percent(800.0, 4), 100.0);
        assert_eq!(normalized_cpu_percent(100.0, 0), 0.0);
        assert_eq!(normalized_cpu_percent(f32::NAN, 4), 0.0);
    }

    #[test]
    fn cross_user_process_is_found_when_executable_access_is_denied() {
        assert!(process_name_matches(
            EXTENSIONS_PROCESS,
            Err(io::ErrorKind::PermissionDenied.into()),
            || Ok("lmm-extensions\n".into()),
        ));
        for comm in [
            "other-service\n",
            "lmm-extensions-worker\n",
            "lmm-extensions \n",
        ] {
            assert!(!process_name_matches(
                EXTENSIONS_PROCESS,
                Err(io::ErrorKind::PermissionDenied.into()),
                || Ok(comm.into()),
            ));
        }
        assert!(!process_name_matches(
            EXTENSIONS_PROCESS,
            Err(io::ErrorKind::PermissionDenied.into()),
            || Err(io::ErrorKind::PermissionDenied.into()),
        ));
    }

    #[test]
    fn comm_does_not_override_executable_identity_or_a_missing_process() {
        for executable in [
            Ok(PathBuf::from("/usr/bin/lmm-extensions")),
            Ok(PathBuf::from("/usr/bin/another-service")),
            Err(io::ErrorKind::NotFound.into()),
        ] {
            let expected = executable
                .as_ref()
                .is_ok_and(|path| path.file_name().unwrap() == EXTENSIONS_PROCESS);
            assert_eq!(
                process_name_matches(EXTENSIONS_PROCESS, executable, || {
                    panic!("comm is only read after a permission denial")
                }),
                expected
            );
        }
    }
}
