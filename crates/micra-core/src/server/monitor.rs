//! Muestreo de CPU y RAM del proceso del servidor para el panel en vivo.

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSample {
    /// Porcentaje sobre la capacidad total del equipo (0–100).
    pub cpu_percent: f32,
    pub memory_mb: u64,
}

pub struct ProcessMonitor {
    sys: System,
    pid: Pid,
    cpus: f32,
}

impl ProcessMonitor {
    pub fn new(pid: u32) -> Self {
        Self {
            sys: System::new(),
            pid: Pid::from_u32(pid),
            cpus: crate::hardware::logical_cpu_count() as f32,
        }
    }

    /// Llamar periódicamente (p. ej. cada segundo). La primera muestra de CPU
    /// es 0 porque sysinfo necesita dos lecturas para calcular el uso.
    pub fn sample(&mut self) -> Option<ResourceSample> {
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        let p = self.sys.process(self.pid)?;
        Some(ResourceSample {
            cpu_percent: (p.cpu_usage() / self.cpus).min(100.0),
            memory_mb: p.memory() / 1024 / 1024,
        })
    }
}
