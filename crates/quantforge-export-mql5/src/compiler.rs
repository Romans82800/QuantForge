use crate::model::{CompileReport, ExportError, MetaEditorConfig};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn compile_with_metaeditor(
    source_path: impl AsRef<Path>,
    config: &MetaEditorConfig,
) -> Result<CompileReport, ExportError> {
    let source_path = absolute_path(source_path.as_ref())?;
    if !source_path.is_file() {
        return Err(ExportError::Compilation(format!(
            "source does not exist: {}",
            source_path.display()
        )));
    }
    if !config.executable.is_file() {
        return Err(ExportError::Compilation(format!(
            "MetaEditor executable does not exist: {}",
            config.executable.display()
        )));
    }
    let binary_path = source_path.with_extension("ex5");
    let log_path = source_path.with_extension("compile.log");
    // MetaEditor under Wine does not reliably parse /compile paths containing
    // spaces, even when Wine quotes the argv item. Compile identical bytes from
    // a space-free temporary path, then publish the binary and log beside the
    // requested source.
    let staging = (config.wine_binary.is_some() && source_path.to_string_lossy().contains(' '))
        .then(tempfile::tempdir)
        .transpose()?;
    let command_source = if let Some(staging) = &staging {
        let path = staging.path().join(
            source_path
                .file_name()
                .ok_or_else(|| ExportError::Compilation("source has no file name".into()))?,
        );
        fs::copy(&source_path, &path)?;
        path
    } else {
        source_path.clone()
    };
    let command_binary = command_source.with_extension("ex5");
    let command_log = command_source.with_extension("compile.log");
    let compile_argument = format!("/compile:{}", command_path(&command_source, config));
    let log_argument = format!("/log:{}", command_path(&command_log, config));

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
    let output = command.arg(compile_argument).arg(log_argument).output()?;

    if staging.is_some() {
        if command_binary.is_file() {
            fs::copy(&command_binary, &binary_path)?;
        }
        if command_log.is_file() {
            fs::copy(&command_log, &log_path)?;
        }
    }
    let log = fs::read(&log_path)
        .map(|bytes| decode_log(&bytes))
        .unwrap_or_default();
    let result_counts = parse_result_counts(&log);
    let errors = result_counts.map(|(errors, _)| errors);
    let warnings = result_counts.map(|(_, warnings)| warnings);
    let success = errors == Some(0) && binary_path.is_file();
    let process_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let log_excerpt = if log.trim().is_empty() {
        tail(&process_output, 40)
    } else {
        tail(&log, 40)
    };

    Ok(CompileReport {
        success,
        process_exit_code: output.status.code(),
        errors,
        warnings,
        source_path,
        binary_path,
        log_path,
        log_excerpt,
    })
}

fn command_path(path: &Path, config: &MetaEditorConfig) -> String {
    if config.wine_binary.is_some() {
        if let Some(prefix) = &config.wine_prefix
            && let Ok(relative) = path.strip_prefix(prefix.join("drive_c"))
        {
            return format!("C:\\{}", relative.to_string_lossy().replace('/', "\\"));
        }
        let unix = path.to_string_lossy().replace('/', "\\");
        format!("Z:{unix}")
    } else {
        path.to_string_lossy().into_owned()
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn decode_log(bytes: &[u8]) -> String {
    let utf16 = bytes.starts_with(&[0xff, 0xfe])
        || bytes
            .iter()
            .skip(1)
            .step_by(2)
            .take(32)
            .filter(|value| **value == 0)
            .count()
            > 8;
    if utf16 {
        let start = usize::from(bytes.starts_with(&[0xff, 0xfe])) * 2;
        let words: Vec<u16> = bytes[start..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&words)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn parse_result_counts(log: &str) -> Option<(usize, usize)> {
    let line = log.lines().rev().find(|line| {
        line.contains("Result:") && line.contains("errors") && line.contains("warnings")
    })?;
    let tokens: Vec<_> = line
        .split(|value: char| value.is_whitespace() || value == ',' || value == ':')
        .filter(|value| !value.is_empty())
        .collect();
    let errors = tokens
        .windows(2)
        .find(|pair| pair[1] == "errors")?
        .first()?
        .parse()
        .ok()?;
    let warnings = tokens
        .windows(2)
        .find(|pair| pair[1] == "warnings")?
        .first()?
        .parse()
        .ok()?;
    Some((errors, warnings))
}

fn tail(value: &str, lines: usize) -> String {
    let values: Vec<_> = value.lines().collect();
    values[values.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_counts_are_parsed_from_metaeditor_log() {
        let log = "one\ntwo\nResult: 0 errors, 2 warnings, 100 msec elapsed";
        assert_eq!(parse_result_counts(log), Some((0, 2)));
    }

    #[test]
    fn wine_paths_use_the_z_drive_mapping() {
        let config = MetaEditorConfig {
            executable: "/tmp/metaeditor64.exe".into(),
            wine_binary: Some("/tmp/wine".into()),
            wine_prefix: None,
        };
        assert_eq!(
            command_path(Path::new("/Users/test/file.mq5"), &config),
            "Z:\\Users\\test\\file.mq5"
        );
    }

    #[test]
    fn paths_inside_the_wine_prefix_use_the_c_drive() {
        let config = MetaEditorConfig {
            executable: "/prefix/drive_c/metaeditor64.exe".into(),
            wine_binary: Some("/tmp/wine".into()),
            wine_prefix: Some("/prefix".into()),
        };
        assert_eq!(
            command_path(Path::new("/prefix/drive_c/Program Files/test.mq5"), &config),
            "C:\\Program Files\\test.mq5"
        );
    }
}
