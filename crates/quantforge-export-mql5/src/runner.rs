use crate::model::{ExportError, TerminalConfig, TesterRunReport};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

pub fn run_mt5_tester(
    tester_ini: impl AsRef<Path>,
    deals_path: impl AsRef<Path>,
    equity_path: impl AsRef<Path>,
    metadata_path: impl AsRef<Path>,
    config: &TerminalConfig,
) -> Result<TesterRunReport, ExportError> {
    let tester_ini = absolute_path(tester_ini.as_ref())?;
    let deals_path = absolute_path(deals_path.as_ref())?;
    let equity_path = absolute_path(equity_path.as_ref())?;
    let metadata_path = absolute_path(metadata_path.as_ref())?;
    if !tester_ini.is_file() {
        return Err(ExportError::Compilation(format!(
            "tester configuration does not exist: {}",
            tester_ini.display()
        )));
    }
    if !config.executable.is_file() {
        return Err(ExportError::Compilation(format!(
            "MT5 terminal executable does not exist: {}",
            config.executable.display()
        )));
    }
    if config.timeout_seconds == 0 {
        return Err(ExportError::InvalidConfig(
            "tester timeout must be greater than zero".into(),
        ));
    }

    let prior_deals = modified(&deals_path);
    let prior_equity = modified(&equity_path);
    let prior_metadata = modified(&metadata_path);
    // terminal64 under Wine ignores /config paths on the Z: drive and silently
    // reuses the last tester job. Stage the identical INI beneath the prefix's
    // drive_c so the command resolves to a native C: path. The directory name
    // is deliberately space-free because terminal64 is also unreliable when
    // the /config argument contains spaces.
    let staging = if config.wine_binary.is_some() {
        let prefix = config.wine_prefix.as_ref().ok_or_else(|| {
            ExportError::Compilation("Wine tester execution requires a WINEPREFIX".into())
        })?;
        let drive_c = prefix.join("drive_c");
        if !drive_c.is_dir() {
            return Err(ExportError::Compilation(format!(
                "Wine prefix has no drive_c directory: {}",
                drive_c.display()
            )));
        }
        Some(tempfile::Builder::new().prefix("qf-tester-").tempdir_in(drive_c)?)
    } else {
        None
    };
    let command_ini = if let Some(staging) = &staging {
        let path = staging.path().join("quantforge-tester.ini");
        fs::copy(&tester_ini, &path)?;
        path
    } else {
        tester_ini.clone()
    };

    let mut command = if let Some(wine) = &config.wine_binary {
        if !wine.is_file() {
            return Err(ExportError::Compilation(format!(
                "Wine binary does not exist: {}",
                wine.display()
            )));
        }
        let mut command = Command::new(wine);
        command.arg(&config.executable);
        if let Some(prefix) = &config.wine_prefix {
            command.env("WINEPREFIX", prefix);
        }
        command.env("WINEDEBUG", "-all");
        command
    } else {
        Command::new(&config.executable)
    };
    command
        .arg(format!("/config:{}", command_path(&command_ini, config)));
    if config.portable {
        command.arg("/portable");
    }
    command.stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(parent) = config.executable.parent() {
        command.current_dir(parent);
    }

    let started = Instant::now();
    let mut child = command.spawn()?;
    let timeout = Duration::from_secs(config.timeout_seconds);
    // LiveUpdate can exit the initial process and respawn terminal64 under a new
    // PID while keeping our /config job. Prefer waiting until parity artifacts are
    // fresh (or timeout), not merely until the first child exits.
    let mut last_status = None;
    let mut child_exited = false;
    let mut grace_applied = false;
    let timed_out = loop {
        if is_fresh(&deals_path, prior_deals)
            && is_fresh(&equity_path, prior_equity)
            && is_fresh(&metadata_path, prior_metadata)
        {
            if !child_exited {
                let _ = child.kill();
                last_status = child.wait().ok();
            }
            break false;
        }
        if !child_exited {
            match child.try_wait() {
                Ok(Some(status)) => {
                    last_status = Some(status);
                    child_exited = true;
                }
                Ok(None) => {}
                Err(error) => return Err(error.into()),
            }
        }
        if child_exited {
            if !native_tester_running() {
                if !grace_applied {
                    grace_applied = true;
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
                break false;
            }
            grace_applied = false;
        }
        if started.elapsed() >= timeout {
            if !child_exited {
                let _ = child.kill();
                last_status = child.wait().ok();
            }
            kill_native_testers();
            break true;
        }
        thread::sleep(Duration::from_millis(500));
    };

    let deals_fresh = is_fresh(&deals_path, prior_deals);
    let equity_fresh = is_fresh(&equity_path, prior_equity);
    let metadata_fresh = is_fresh(&metadata_path, prior_metadata);
    Ok(TesterRunReport {
        success: !timed_out && deals_fresh && equity_fresh && metadata_fresh,
        timed_out,
        process_exit_code: last_status.and_then(|status| status.code()),
        elapsed_milliseconds: started.elapsed().as_millis(),
        tester_ini,
        deals_path,
        equity_path,
        metadata_path,
        deals_fresh,
        equity_fresh,
        metadata_fresh,
    })
}

fn native_tester_running() -> bool {
    #[cfg(windows)]
    {
        for name in ["terminal64.exe", "metatester64.exe"] {
            if let Ok(output) = Command::new("tasklist")
                .args(["/FI", &format!("IMAGENAME eq {name}"), "/NH"])
                .output()
            {
                let text = String::from_utf8_lossy(&output.stdout);
                if text.to_ascii_lowercase().contains(&name.to_ascii_lowercase()) {
                    return true;
                }
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn kill_native_testers() {
    #[cfg(windows)]
    {
        for name in ["terminal64.exe", "metatester64.exe"] {
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn command_path(path: &Path, config: &TerminalConfig) -> String {
    if config.wine_binary.is_some() {
        let unix = path.to_string_lossy().replace('/', "\\");
        format!("Z:{unix}")
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn is_fresh(path: &Path, previous: Option<SystemTime>) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if metadata.len() == 0 {
        return false;
    }
    match (previous, metadata.modified().ok()) {
        (Some(previous), Some(current)) => current > previous,
        (None, Some(_)) => true,
        _ => false,
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
