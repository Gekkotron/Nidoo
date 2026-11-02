//! Machine sampler: CPU, memory, disk and temperature of the host, plus the
//! hub process's own share of CPU and memory.

use std::path::{Path, PathBuf};

use nidoo_api::{CpuView, MemView};
use sysinfo::{
    Components, Disks, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind,
    System,
};

/// One reading of the machine; a figure that cannot be measured is `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct Resources {
    pub cpu: Option<CpuView>,
    pub ram: Option<MemView>,
    /// Used and total bytes of the disk holding the data directory.
    pub disk: Option<(u64, u64)>,
    pub temperature_c: Option<f64>,
}

/// Keeps the `sysinfo` state between calls so CPU % covers the last interval.
pub struct Sampler {
    /// `None` when the platform offers no system information.
    system: Option<System>,
    data_dir: PathBuf,
    pid: Option<Pid>,
    primed: bool,
}

impl Sampler {
    pub fn new(data_dir: PathBuf) -> Sampler {
        let system = System::new_with_specifics(
            RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
        )
        .ok();
        Sampler {
            system,
            data_dir,
            pid: sysinfo::get_current_pid().ok(),
            primed: false,
        }
    }

    /// Reads the machine now. The first call has no previous interval, so its
    /// CPU figure is `None`; anything the platform cannot report is `None`.
    pub fn sample(&mut self) -> Resources {
        let (cpu, ram) = self.read_system();
        Resources {
            cpu,
            ram,
            disk: self.read_disk(),
            temperature_c: read_temperature(),
        }
    }

    fn read_system(&mut self) -> (Option<CpuView>, Option<MemView>) {
        let Some(system) = self.system.as_mut() else {
            return (None, None);
        };
        system.refresh_cpu_usage();
        system.refresh_memory();
        if let Some(pid) = self.pid {
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing().with_cpu().with_memory(),
            );
        }
        let process = self.pid.and_then(|pid| system.process(pid));

        let cpu = self.primed.then(|| CpuView {
            server_pct: f64::from(system.global_cpu_usage()).clamp(0.0, 100.0),
            hub_pct: process.map_or(0.0, |p| hub_share(p.cpu_usage(), system.cpus().len())),
        });
        self.primed = true;

        let total = system.total_memory();
        let ram = (total > 0).then(|| MemView {
            used_bytes: system.used_memory().min(total),
            total_bytes: total,
            hub_bytes: process.map_or(0, |p| p.memory()),
        });
        (cpu, ram)
    }

    fn read_disk(&self) -> Option<(u64, u64)> {
        let disks = Disks::new_with_refreshed_list().ok()?;
        let mounts: Vec<(PathBuf, u64, u64)> = disks
            .iter()
            .map(|d| {
                (
                    d.mount_point().to_path_buf(),
                    d.total_space(),
                    d.available_space(),
                )
            })
            .collect();
        disk_for(&self.data_dir, &mounts)
    }
}

fn read_temperature() -> Option<f64> {
    let components = Components::new_with_refreshed_list().ok()?;
    let readings: Vec<(String, f32)> = components
        .iter()
        .filter_map(|c| c.temperature().map(|t| (c.label().to_string(), t)))
        .collect();
    cpu_temperature(&readings)
}

/// The process's CPU as a share of the whole machine: `sysinfo` reports 100
/// per fully used core, so divide by the core count and clamp to 0-100.
pub fn hub_share(process_pct_of_one_core: f32, cpus: usize) -> f64 {
    if cpus == 0 {
        return 0.0;
    }
    (f64::from(process_pct_of_one_core) / cpus as f64).clamp(0.0, 100.0)
}

/// Used and total bytes of the disk whose mount point is the longest prefix of
/// `path`; `None` when no mount matches or the disk reports a zero size.
pub fn disk_for(path: &Path, disks: &[(PathBuf, u64, u64)]) -> Option<(u64, u64)> {
    disks
        .iter()
        .filter(|(mount, _, _)| path.starts_with(mount))
        .max_by_key(|(mount, _, _)| mount.components().count())
        .filter(|(_, total, _)| *total > 0)
        .map(|(_, total, available)| (total.saturating_sub(*available), *total))
}

/// The CPU temperature: the hottest reading whose label names a CPU sensor,
/// else the hottest of all. `None` when there is no usable reading.
pub fn cpu_temperature(components: &[(String, f32)]) -> Option<f64> {
    const CPU_LABELS: [&str; 5] = ["cpu", "package", "coretemp", "k10temp", "soc"];
    let valid: Vec<(&str, f64)> = components
        .iter()
        .filter(|(_, t)| t.is_finite() && *t > 0.0)
        .map(|(label, t)| (label.as_str(), f64::from(*t)))
        .collect();
    let hottest = |readings: &mut dyn Iterator<Item = f64>| readings.reduce(f64::max);
    let cpu = hottest(
        &mut valid
            .iter()
            .filter(|(label, _)| {
                let label = label.to_lowercase();
                CPU_LABELS.iter().any(|k| label.contains(k))
            })
            .map(|(_, t)| *t),
    );
    cpu.or_else(|| hottest(&mut valid.iter().map(|(_, t)| *t)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_share_divides_by_cpus() {
        assert_eq!(hub_share(200.0, 4), 50.0);
        assert_eq!(hub_share(500.0, 4), 100.0);
        assert_eq!(hub_share(10.0, 0), 0.0);
    }

    #[test]
    fn disk_for_picks_longest_mount() {
        let disks = vec![
            (PathBuf::from("/"), 1000, 400),
            (PathBuf::from("/data"), 500, 100),
        ];
        assert_eq!(disk_for(Path::new("/data"), &disks), Some((400, 500)));
    }

    #[test]
    fn disk_for_without_match_is_none() {
        let disks = vec![(PathBuf::from("/boot"), 1000, 400)];
        assert_eq!(disk_for(Path::new("/data"), &disks), None);
    }

    #[test]
    fn disk_for_zero_total_is_none() {
        let disks = vec![(PathBuf::from("/data"), 0, 0)];
        assert_eq!(disk_for(Path::new("/data"), &disks), None);
    }

    #[test]
    fn temperature_prefers_cpu_labels() {
        let c = vec![
            ("acpitz".to_string(), 40.0),
            ("coretemp Package id 0".to_string(), 55.0),
        ];
        assert_eq!(cpu_temperature(&c), Some(55.0));
    }

    #[test]
    fn temperature_falls_back_to_hottest() {
        let c = vec![("acpitz".to_string(), 40.0), ("nvme".to_string(), 45.0)];
        assert_eq!(cpu_temperature(&c), Some(45.0));
    }

    #[test]
    fn temperature_none_when_empty() {
        assert_eq!(cpu_temperature(&[]), None);
        assert_eq!(cpu_temperature(&[("cpu".to_string(), f32::NAN)]), None);
        assert_eq!(cpu_temperature(&[("cpu".to_string(), 0.0)]), None);
    }

    #[test]
    fn sample_reads_this_machine() {
        let mut sampler = Sampler::new(std::env::temp_dir());
        let _ = sampler.sample();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let r = sampler.sample();
        let ram = r.ram.expect("ram");
        assert!(ram.used_bytes <= ram.total_bytes);
        assert!(ram.hub_bytes > 0);
        assert!(r.cpu.is_some());
    }
}
