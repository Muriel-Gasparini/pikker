use crate::runtime::IntegrationError;
use std::{fmt, io::Read, path::Path};
#[derive(Clone, PartialEq, Eq)]
pub struct LcuCredentials {
    pub(crate) pid: u32,
    pub(crate) port: u16,
    pub(crate) token: String,
}
impl fmt::Debug for LcuCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LcuCredentials")
            .field("pid", &self.pid)
            .field("port", &self.port)
            .field("token", &"[REDACTED]")
            .finish()
    }
}
pub fn parse_lockfile(text: &str) -> Result<LcuCredentials, IntegrationError> {
    let invalid = || IntegrationError::Invalid("Lockfile LCU inválido");
    if text.len() > 4096 {
        return Err(invalid());
    }
    let fields: Vec<_> = text.trim_end_matches(['\r', '\n']).split(':').collect();
    if fields.len() != 5
        || !matches!(fields[0], "LeagueClient" | "LeagueClientUx")
        || fields[4] != "https"
    {
        return Err(invalid());
    }
    let pid: u32 = fields[1].parse().map_err(|_| invalid())?;
    let port: u16 = fields[2].parse().map_err(|_| invalid())?;
    let token = fields[3];
    if pid == 0
        || port == 0
        || token.is_empty()
        || token.len() > 256
        || !token.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(invalid());
    }
    Ok(LcuCredentials {
        pid,
        port,
        token: token.to_owned(),
    })
}
pub fn discover(
    configured_path: Option<&Path>,
) -> Result<Option<LcuCredentials>, IntegrationError> {
    if let Some(path) = configured_path {
        return read_lockfile(path);
    }
    #[cfg(windows)]
    for path in windows_paths()? {
        if let Some(credentials) = read_lockfile(&path)? {
            return Ok(Some(credentials));
        }
    }
    Ok(None)
}
fn read_lockfile(path: &Path) -> Result<Option<LcuCredentials>, IntegrationError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut text = String::new();
    file.take(4097).read_to_string(&mut text)?;
    parse_lockfile(&text).map(Some)
}
pub fn lockfile_candidates(images: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    images
        .iter()
        .filter(|image| {
            image.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.eq_ignore_ascii_case("LeagueClient.exe")
                    || n.eq_ignore_ascii_case("LeagueClientUx.exe")
            })
        })
        .filter_map(|image| image.parent().map(|parent| parent.join("lockfile")))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
#[cfg(windows)]
fn windows_paths() -> Result<Vec<std::path::PathBuf>, IntegrationError> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::*,
            Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    };
    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    // SAFETY: Toolhelp writes only the initialized PROCESSENTRY32W sized by dwSize;
    // every successful native handle has exactly one owner and is closed on all paths.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    let snapshot = OwnedHandle(snapshot);
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut present = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    let mut images = Vec::new();
    while present {
        let name = String::from_utf16_lossy(
            &entry.szExeFile[..entry
                .szExeFile
                .iter()
                .position(|v| *v == 0)
                .unwrap_or(entry.szExeFile.len())],
        );
        if name.eq_ignore_ascii_case("LeagueClient.exe")
            || name.eq_ignore_ascii_case("LeagueClientUx.exe")
        {
            let handle =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID) };
            if !handle.is_null() {
                let handle = OwnedHandle(handle);
                let mut buffer = vec![0u16; 32768];
                let mut length = buffer.len() as u32;
                if unsafe {
                    QueryFullProcessImageNameW(handle.0, 0, buffer.as_mut_ptr(), &mut length)
                } != 0
                    && length as usize <= buffer.len()
                {
                    use std::os::windows::ffi::OsStringExt;
                    images.push(std::path::PathBuf::from(std::ffi::OsString::from_wide(
                        &buffer[..length as usize],
                    )));
                }
            }
        }
        present = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    Ok(lockfile_candidates(&images))
}
