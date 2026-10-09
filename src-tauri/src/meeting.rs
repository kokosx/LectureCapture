//! Scheduled end of a recording ("finish at 21:00") and leaving the Teams meeting by
//! sending its keyboard shortcut (macOS ⌘⇧H, Windows Ctrl+Shift+H) to the meeting window.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local, NaiveTime, TimeZone};
use lc_capture::CaptureTarget;
use serde::Serialize;

/// Teams window the shortcut is sent to (taken from the capture target when possible).
#[derive(Clone, Debug, Default, Serialize)]
pub struct MeetingWindow {
    /// macOS bundle id / Windows executable name.
    pub app: Option<String>,
    pub title: Option<String>,
}

impl MeetingWindow {
    pub fn from_target(t: &CaptureTarget) -> Self {
        match t {
            CaptureTarget::Window { bundle_id, title, .. } => Self { app: bundle_id.clone(), title: title.clone() },
            CaptureTarget::Display { .. } => Self::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AutoStop {
    /// RFC 3339 local time.
    pub at: String,
    #[serde(skip)]
    pub at_time: DateTime<Local>,
    pub leave_meeting: bool,
    pub window: MeetingWindow,
}

/// Next occurrence of `HH:MM` (today, or tomorrow if it already passed).
pub fn next_occurrence(hhmm: &str, now: DateTime<Local>) -> Result<DateTime<Local>> {
    let t = NaiveTime::parse_from_str(hhmm.trim(), "%H:%M").with_context(|| format!("Nieprawidłowa godzina „{hhmm}” (użyj GG:MM)."))?;
    let mut day = now.date_naive();
    for _ in 0..3 {
        if let Some(dt) = Local.from_local_datetime(&day.and_time(t)).earliest() {
            if dt > now {
                return Ok(dt);
            }
        }
        day = day.succ_opt().context("date overflow")?;
    }
    bail!("Nie można ustawić godziny {hhmm}.")
}

pub fn shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘⇧H"
    } else {
        "Ctrl+Shift+H"
    }
}

/// Whether this app may send keystrokes (macOS Accessibility). Always true elsewhere.
pub fn can_send_keys() -> bool {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        unsafe { AXIsProcessTrusted() }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Opens the system settings page where keystroke access is granted (macOS).
pub fn open_key_permission_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

#[cfg(target_os = "macos")]
const MAC_SCRIPT: &[&str] = &[
    "on run argv",
    "set appId to item 1 of argv",
    "set winTitle to item 2 of argv",
    "tell application \"System Events\"",
    "set procs to {}",
    "if appId is not \"\" then set procs to (every application process whose bundle identifier is appId)",
    "if (count of procs) is 0 then set procs to (every application process whose bundle identifier is \"com.microsoft.teams2\")",
    "if (count of procs) is 0 then set procs to (every application process whose bundle identifier is \"com.microsoft.teams\")",
    "if (count of procs) is 0 then error \"Nie znaleziono uruchomionej aplikacji Microsoft Teams.\"",
    "set p to item 1 of procs",
    "set frontmost of p to true",
    "if winTitle is not \"\" then",
    "try",
    "perform action \"AXRaise\" of (first window of p whose name is winTitle)",
    "end try",
    "end if",
    "end tell",
    "delay 0.7",
    "tell application \"System Events\" to keystroke \"h\" using {command down, shift down}",
    "end run",
];

/// Bring the Teams meeting window to the front and press the "leave" shortcut.
pub fn leave_meeting(w: &MeetingWindow) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if !can_send_keys() {
            bail!("Brak uprawnienia „Dostępność” – nadaj je aplikacji LectureCapture w Ustawieniach systemowych → Prywatność i ochrona → Dostępność.");
        }
        let mut cmd = std::process::Command::new("osascript");
        for line in MAC_SCRIPT {
            cmd.arg("-e").arg(line);
        }
        cmd.arg(w.app.clone().unwrap_or_default()).arg(w.title.clone().unwrap_or_default());
        let out = cmd.output().context("osascript")?;
        if !out.status.success() {
            bail!("Nie udało się wysłać skrótu do Teams: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let script = r#"
$ws = New-Object -ComObject WScript.Shell
$ok = $false
if ($env:LC_TITLE) { $ok = $ws.AppActivate($env:LC_TITLE) }
if (-not $ok) {
  $names = @('ms-teams', 'Teams')
  if ($env:LC_APP) { $names = @($env:LC_APP -replace '\.exe$', '') + $names }
  $p = Get-Process -Name $names -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  if (-not $p) { Write-Error 'Nie znaleziono okna Microsoft Teams.'; exit 2 }
  $ok = $ws.AppActivate($p.Id)
}
Start-Sleep -Milliseconds 700
$ws.SendKeys('^+h')
"#;
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
            .env("LC_TITLE", w.title.clone().unwrap_or_default())
            .env("LC_APP", w.app.clone().unwrap_or_default())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .context("powershell")?;
        if !out.status.success() {
            bail!("Nie udało się wysłać skrótu do Teams: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = w;
        bail!("Opuszczanie spotkania nie jest obsługiwane na tym systemie.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_occurrence_today_or_tomorrow() {
        let now = Local.with_ymd_and_hms(2026, 10, 9, 20, 15, 0).unwrap();
        let a = next_occurrence("21:00", now).unwrap();
        assert_eq!(a, Local.with_ymd_and_hms(2026, 10, 9, 21, 0, 0).unwrap());
        let b = next_occurrence("08:30", now).unwrap();
        assert_eq!(b, Local.with_ymd_and_hms(2026, 10, 10, 8, 30, 0).unwrap());
        assert!(next_occurrence("25:00", now).is_err());
        assert!(next_occurrence("abc", now).is_err());
    }
}
