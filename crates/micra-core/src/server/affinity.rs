//! Afinidad de CPU del proceso del servidor.
//!
//! - Linux: `sched_setaffinity` en el hijo justo antes del `exec` (es lo mismo
//!   que hace `taskset`, sin depender de que esté instalado). La JVM hereda la
//!   máscara y además ajusta su número de hilos de GC a los núcleos visibles.
//! - Windows: `SetProcessAffinityMask` sobre el handle del proceso recién
//!   creado. Solo cubre el primer grupo de procesadores (64 CPUs lógicas),
//!   suficiente para equipos de escritorio.

/// Máscara de bits para una lista de CPUs (CPUs >= 64 se ignoran).
pub fn cores_to_mask(cores: &[usize]) -> u64 {
    cores
        .iter()
        .filter(|&&c| c < 64)
        .fold(0u64, |mask, &c| mask | (1u64 << c))
}

#[cfg(target_os = "linux")]
pub fn apply_before_spawn(cmd: &mut tokio::process::Command, cores: &[usize]) {
    let cores = cores.to_vec();
    // SAFETY: el cierre se ejecuta en el hijo tras `fork`; solo llama a
    // funciones async-signal-safe y no reserva memoria (el Vec ya existe).
    unsafe {
        cmd.pre_exec(move || {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_ZERO(&mut set);
            for &c in &cores {
                libc::CPU_SET(c, &mut set);
            }
            if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
pub fn apply_before_spawn(_cmd: &mut tokio::process::Command, _cores: &[usize]) {}

#[cfg(windows)]
pub fn apply_after_spawn(child: &tokio::process::Child, cores: &[usize]) -> std::io::Result<()> {
    use windows_sys::Win32::System::Threading::SetProcessAffinityMask;

    let Some(handle) = child.raw_handle() else {
        return Err(std::io::Error::other("el proceso ya terminó"));
    };
    let mask = cores_to_mask(cores) as usize;
    // SAFETY: el handle pertenece a `child`, que sigue vivo durante la llamada.
    let ok = unsafe { SetProcessAffinityMask(handle as _, mask) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn apply_after_spawn(_child: &tokio::process::Child, _cores: &[usize]) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::cores_to_mask;

    #[test]
    fn mask() {
        assert_eq!(cores_to_mask(&[0, 1, 3]), 0b1011);
        assert_eq!(cores_to_mask(&[70]), 0);
    }
}
