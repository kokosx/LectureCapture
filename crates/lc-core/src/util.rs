//! Small helpers: slugs, durations, process liveness.

/// ASCII slug safe on macOS and Windows file systems (Polish diacritics folded).
pub fn slugify(input: &str, max_len: usize) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in input.chars() {
        let mapped: Option<&str> = match ch {
            'ą' => Some("a"), 'Ą' => Some("A"), 'ć' => Some("c"), 'Ć' => Some("C"),
            'ę' => Some("e"), 'Ę' => Some("E"), 'ł' => Some("l"), 'Ł' => Some("L"),
            'ń' => Some("n"), 'Ń' => Some("N"), 'ó' => Some("o"), 'Ó' => Some("O"),
            'ś' => Some("s"), 'Ś' => Some("S"), 'ź' | 'ż' => Some("z"), 'Ź' | 'Ż' => Some("Z"),
            'ä' => Some("a"), 'ö' => Some("o"), 'ü' => Some("u"), 'ß' => Some("ss"),
            'é' | 'è' | 'ê' => Some("e"), 'á' | 'à' => Some("a"), 'í' => Some("i"), 'ú' => Some("u"),
            _ => None,
        };
        if let Some(m) = mapped {
            out.push_str(m);
            dash = false;
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= max_len {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "Wyklad".to_string()
    } else {
        trimmed
    }
}

/// `H:MM:SS` (or `MM:SS` below one hour).
pub fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Human-friendly duration, e.g. `2 h 05 min`.
pub fn fmt_duration_long(ms: u64) -> String {
    let total_min = ms / 60_000;
    let (h, m) = (total_min / 60, total_min % 60);
    if h > 0 {
        format!("{h} h {m:02} min")
    } else {
        format!("{m} min {} s", (ms / 1000) % 60)
    }
}

/// Whether a process with this PID is currently alive.
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) checks existence without sending a signal.
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code: u32 = 0;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            ok && code == STILL_ACTIVE as u32
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// Free space (bytes) on the volume containing `path`.
pub fn free_space(path: &std::path::Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        Some(st.f_bavail as u64 * st.f_frsize as u64)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut avail: u64 = 0;
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, std::ptr::null_mut(), std::ptr::null_mut()) };
        if ok == 0 { None } else { Some(avail) }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_folds_polish() {
        assert_eq!(slugify("Algorytmy i struktury danych", 60), "Algorytmy-i-struktury-danych");
        assert_eq!(slugify("Źródła światła: łącza!", 60), "Zrodla-swiatla-lacza");
        assert_eq!(slugify("  ///  ", 60), "Wyklad");
        assert_eq!(slugify("a/b\\c:d*e?f\"g<h>i|j", 60), "a-b-c-d-e-f-g-h-i-j");
    }

    #[test]
    fn fmt() {
        assert_eq!(fmt_ms(65_000), "01:05");
        assert_eq!(fmt_ms(3_725_000), "1:02:05");
        assert_eq!(fmt_duration_long(7_500_000), "2 h 05 min");
    }

    #[test]
    fn free_space_of_temp_dir() {
        assert!(free_space(&std::env::temp_dir()).unwrap_or(0) > 0);
    }

    #[test]
    fn own_pid_alive() {
        assert!(pid_alive(std::process::id()));
    }
}
