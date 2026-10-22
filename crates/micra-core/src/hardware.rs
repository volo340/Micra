use serde::Serialize;
use sysinfo::{MemoryRefreshKind, RefreshKind, System};

/// Datos de la máquina que necesita el asistente de creación para ofrecer
/// límites sensatos de RAM y la lista de núcleos para la afinidad.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareInfo {
    pub logical_cpus: usize,
    pub total_memory_mb: u64,
    /// Máximo recomendado para el servidor: deja al menos 2 GB (o un 25 %)
    /// al sistema operativo.
    pub recommended_max_mb: u64,
}

pub fn hardware_info() -> HardwareInfo {
    let sys = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let total_memory_mb = sys.total_memory() / 1024 / 1024;
    let reserve = (total_memory_mb / 4).max(2048);
    HardwareInfo {
        logical_cpus: logical_cpu_count(),
        total_memory_mb,
        recommended_max_mb: total_memory_mb.saturating_sub(reserve).max(1024),
    }
}

pub fn logical_cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
