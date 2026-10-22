//! Aislamiento en Windows: Job Object y, opcionalmente, integridad baja.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW,
    SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    GetLengthSid, GetSecurityDescriptorSacl, SetTokenInformation, TokenIntegrityLevel, ACL,
    LABEL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SID_AND_ATTRIBUTES,
    TOKEN_ADJUST_DEFAULT, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicUIRestrictions,
    JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_BASIC_UI_RESTRICTIONS,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
};
use windows_sys::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
use windows_sys::Win32::System::Threading::{
    OpenProcessToken, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};

/// SID del nivel de integridad bajo.
const LOW_INTEGRITY_SID: &str = "S-1-16-4096";
/// Etiqueta "integridad baja, heredable" para la carpeta del servidor.
const LOW_LABEL_SDDL: &str = "S:(ML;OICI;NW;;;LW)";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Handle de un Job Object. Mientras exista, el servidor sigue vivo; al
/// cerrarse (también si Micra se cierra de golpe) Windows lo termina.
pub struct Job(HANDLE);

// SAFETY: un HANDLE de Job Object se puede usar desde cualquier hilo.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: el handle es nuestro y se cierra una sola vez.
        unsafe { CloseHandle(self.0) };
    }
}

pub fn create_job() -> io::Result<Job> {
    // SAFETY: sin nombre ni atributos de seguridad.
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = Job(handle);

    // SAFETY: estructura de datos simple, toda a ceros es válida.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: puntero y tamaño de una estructura válida.
    let ok = unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_EXITWINDOWS,
    };
    // SAFETY: puntero y tamaño de una estructura válida.
    let ok = unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectBasicUIRestrictions,
            &ui as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

/// Mete el proceso en el Job Object.
///
/// # Safety
/// `process` debe ser un handle de proceso válido durante la llamada.
pub unsafe fn assign(job: &Job, process: HANDLE) -> io::Result<()> {
    // SAFETY: el job es nuestro y quien llama garantiza `process`.
    if unsafe { AssignProcessToJobObject(job.0, process) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Baja el nivel de integridad del token de un proceso recién creado (y
/// todavía suspendido). Bajar siempre está permitido; subir, no.
///
/// # Safety
/// `process` debe ser un handle de proceso válido durante la llamada.
pub unsafe fn lower_integrity(process: HANDLE) -> io::Result<()> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `process` es válido y `token` recibe un handle nuevo.
    if unsafe { OpenProcessToken(process, TOKEN_ADJUST_DEFAULT | TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut sid: PSID = std::ptr::null_mut();
    let sid_str = wide(LOW_INTEGRITY_SID);
    // SAFETY: cadena terminada en nulo; el SID se libera con LocalFree.
    if unsafe { ConvertStringSidToSidW(sid_str.as_ptr(), &mut sid) } == 0 {
        let err = io::Error::last_os_error();
        // SAFETY: handle abierto arriba.
        unsafe { CloseHandle(token) };
        return Err(err);
    }
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: sid,
            Attributes: SE_GROUP_INTEGRITY as u32,
        },
    };
    // Como en el ejemplo de Microsoft: tamaño de la estructura más el del SID.
    // SAFETY: `sid` es un SID válido.
    let size = std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + unsafe { GetLengthSid(sid) };
    // SAFETY: estructura válida; el SID vive hasta el LocalFree de abajo.
    let ok = unsafe {
        SetTokenInformation(
            token,
            TokenIntegrityLevel,
            &label as *const _ as *const core::ffi::c_void,
            size,
        )
    };
    let err = (ok == 0).then(io::Error::last_os_error);
    // SAFETY: liberar lo reservado por ConvertStringSidToSidW y cerrar el token.
    unsafe {
        LocalFree(sid);
        CloseHandle(token);
    }
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Reanuda el hilo principal de un proceso creado con `CREATE_SUSPENDED`.
pub fn resume_process(pid: u32) -> io::Result<()> {
    // SAFETY: instantánea de hilos de todo el sistema; se cierra al final.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: estructura simple; dwSize se rellena como pide la API.
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
    let mut resumed = 0;
    // SAFETY: snapshot válido y entrada inicializada.
    let mut more = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: id de hilo obtenido de la instantánea.
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !thread.is_null() {
                // SAFETY: handle de hilo válido; se cierra justo después.
                unsafe {
                    ResumeThread(thread);
                    CloseHandle(thread);
                }
                resumed += 1;
            }
        }
        // SAFETY: igual que arriba.
        more = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }
    // SAFETY: cerrar la instantánea.
    unsafe { CloseHandle(snapshot) };
    if resumed == 0 {
        return Err(io::Error::other("no se encontró el hilo principal"));
    }
    Ok(())
}

/// Marca la carpeta del servidor como de integridad baja (heredable), para
/// que un proceso con integridad baja pueda escribir en ella.
pub fn label_low_integrity(dir: &Path) -> io::Result<()> {
    let sddl = wide(LOW_LABEL_SDDL);
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: cadena terminada en nulo; el descriptor se libera con LocalFree.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut present = 0;
    let mut defaulted = 0;
    let mut sacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: descriptor válido recién creado.
    let ok = unsafe { GetSecurityDescriptorSacl(sd, &mut present, &mut sacl, &mut defaulted) };
    let result = if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        let path = wide_path(dir);
        // SAFETY: ruta terminada en nulo y SACL dentro de `sd`, que sigue vivo.
        let status = unsafe {
            SetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                LABEL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
                sacl,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status as i32))
        }
    };
    // SAFETY: liberar el descriptor reservado por la conversión.
    unsafe { LocalFree(sd) };
    result
}
