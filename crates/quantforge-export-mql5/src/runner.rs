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
    // Like MetaEditor, terminal64 under Wine is unreliable when /config
    // contains spaces. Stage identical INI bytes at a space-free path.
    let staging = config
        .wine_binary
        .is_some()
        .then(tempfile::tempdir)
        .transpose()?;
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
        .arg(format!("/config:{}", command_path(&command_ini, config)))
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let started = Instant::now();
    let mut child = command.spawn()?;
    let timeout = Duration::from_secs(config.timeout_seconds);
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (Some(status), false);
        }
        if started.elapsed() >= timeout {
            child.kill()?;
            let status = child.wait().ok();
            break (status, true);
        }
        thread::sleep(Duration::from_millis(250));
    };

    let deals_fresh = is_fresh(&deals_path, prior_deals);
    let equity_fresh = is_fresh(&equity_path, prior_equity);
    let metadata_fresh = is_fresh(&metadata_path, prior_metadata);
    Ok(TesterRunReport {
        success: !timed_out && deals_fresh && equity_fresh && metadata_fresh,
        timed_out,
        process_exit_code: status.and_then(|status| status.code()),
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
