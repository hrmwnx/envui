use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::app::{EnvEntry, EnvSource};

static TEMP_FILE_ID: AtomicUsize = AtomicUsize::new(0);

/// Check whether the current process has an elevated Windows administrator token.
#[cfg(windows)]
pub fn has_admin_privileges() -> bool {
    // IsUserAnAdmin checks the effective token, so a UAC-filtered process is
    // treated as non-administrator even when its account belongs to that group.
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

#[cfg(not(windows))]
pub fn has_admin_privileges() -> bool {
    true
}

/// Read the project dotenv file, or enumerate persistent Windows scopes when
/// no dotenv file is present.
pub fn load_env(path: impl AsRef<Path>) -> io::Result<Vec<EnvEntry>> {
    #[cfg(windows)]
    {
        let _ = path;
        load_windows_environment()
    }
    #[cfg(not(windows))]
    {
        let path = path.as_ref();
        if path.exists() {
            load_dotenv(path)
        } else {
            let mut entries: Vec<_> = std::env::vars()
                .map(|(key, value)| EnvEntry {
                    key,
                    value,
                    source: EnvSource::Process,
                    expandable: false,
                    leading_lines: Vec::new(),
                    trailing_lines: Vec::new(),
                })
                .collect();
            entries.sort_by(|left, right| left.key.cmp(&right.key));
            Ok(entries)
        }
    }
}

/// Read a `.env` file explicitly. On Windows this is selected with `--dotenv`;
/// normal startup reads User and System variables from their registry scopes.
pub fn load_dotenv(path: impl AsRef<Path>) -> io::Result<Vec<EnvEntry>> {
    let path = path.as_ref();
    if path.exists() {
        let parsed: Result<Vec<(String, String)>, dotenvy::Error> =
            dotenvy::from_path_iter(path).and_then(|entries| entries.collect());
        match parsed {
            Ok(entries) => {
                let entries = entries
                    .into_iter()
                    .map(|(key, value)| EnvEntry {
                        key,
                        value,
                        source: EnvSource::Dotenv,
                        expandable: false,
                        leading_lines: Vec::new(),
                        trailing_lines: Vec::new(),
                    })
                    .collect();
                let contents = fs::read_to_string(path)?;
                Ok(apply_dotenv_layout(entries, &contents))
            }
            Err(dotenv_error) => {
                // Windows permits names such as `CommonProgramFiles(x86)`, but
                // dotenvy only accepts shell-style keys. Retry with a temporary
                // valid key so saved Windows environment files can be reopened.
                let contents = fs::read_to_string(path)?;
                parse_compatible_env(&contents)
                    .map(|entries| apply_dotenv_layout(entries, &contents))
                    .map_err(|fallback_error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("{dotenv_error}; compatible parser: {fallback_error}"),
                        )
                    })
            }
        }
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("dotenv file not found: {}", path.display()),
        ))
    }
}

fn apply_dotenv_layout(mut entries: Vec<EnvEntry>, contents: &str) -> Vec<EnvEntry> {
    let mut pending_lines = Vec::new();
    let mut entry_index = 0;
    for line in contents.lines() {
        let matches_next_entry = dotenv_assignment_key(line).is_some_and(|key| {
            entries
                .get(entry_index)
                .is_some_and(|entry| entry.key == key)
        });
        if matches_next_entry {
            entries[entry_index].leading_lines = std::mem::take(&mut pending_lines);
            entry_index += 1;
        } else {
            pending_lines.push(line.to_owned());
        }
    }
    if let Some(last) = entries.last_mut() {
        last.trailing_lines = pending_lines;
    }
    entries
}

fn parse_compatible_env(contents: &str) -> io::Result<Vec<EnvEntry>> {
    let mut entries = Vec::new();
    for (line_number, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let assignment = line.strip_prefix("export ").unwrap_or(line);
        let (key, raw_value) = assignment.split_once('=').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid assignment on line {}", line_number + 1),
            )
        })?;
        let key = key.trim();
        if key.is_empty() || key.chars().any(char::is_whitespace) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid key on line {}", line_number + 1),
            ));
        }

        let normalized = format!("ENVUI_VALUE={}", raw_value.trim());
        let mut parsed = dotenvy::from_read_iter(std::io::Cursor::new(normalized));
        let (_, value) = parsed
            .next()
            .transpose()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("missing value on line {}", line_number + 1),
                )
            })?;
        entries.push(EnvEntry {
            key: key.to_owned(),
            value,
            source: EnvSource::Dotenv,
            expandable: false,
            leading_lines: Vec::new(),
            trailing_lines: Vec::new(),
        });
    }
    Ok(entries)
}

/// Write entries as dotenv assignments using a temporary sibling file and rename.
pub fn save_env(path: impl AsRef<Path>, entries: &[EnvEntry]) -> io::Result<()> {
    let mut contents = String::new();
    for entry in entries {
        use std::fmt::Write as _;
        writeln!(
            &mut contents,
            "{}=\"{}\"",
            entry.key,
            escape_dotenv_value(&entry.value)
        )
        .expect("writing to a String cannot fail");
    }
    write_atomically(path.as_ref(), contents.as_bytes())
}

/// Update dotenv assignments while retaining comments and blank lines from the
/// existing file. Removed variables lose their assignment line; new variables
/// are appended after the existing document.
fn save_dotenv_changes(path: impl AsRef<Path>, current: &[EnvEntry]) -> io::Result<()> {
    let path = path.as_ref();
    let contents = fs::read_to_string(path)?;
    let mut output = String::new();
    let mut written = vec![false; current.len()];

    for line in contents.lines() {
        if let Some(key) = dotenv_assignment_key(line) {
            if let Some((index, entry)) = current
                .iter()
                .enumerate()
                .find(|(index, entry)| !written[*index] && entry.key == key)
            {
                use std::fmt::Write as _;
                writeln!(
                    &mut output,
                    "{}=\"{}\"",
                    entry.key,
                    escape_dotenv_value(&entry.value)
                )
                .expect("writing to a String cannot fail");
                written[index] = true;
            }
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    for (index, entry) in current.iter().enumerate() {
        if !written[index] {
            use std::fmt::Write as _;
            writeln!(
                &mut output,
                "{}=\"{}\"",
                entry.key,
                escape_dotenv_value(&entry.value)
            )
            .expect("writing to a String cannot fail");
        }
    }

    write_atomically(path, output.as_bytes())
}

fn dotenv_assignment_key(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let assignment = line.strip_prefix("export ").unwrap_or(line);
    let (key, _) = assignment.split_once('=')?;
    Some(key.trim())
}

fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "dotenv path must name a file")
    })?;

    let mut temp_path = PathBuf::new();
    let mut temp_file = None;
    for _ in 0..100 {
        let id = TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        temp_path = parent.join(format!(
            ".{}.{}.{}.tmp",
            file_name.to_string_lossy(),
            std::process::id(),
            id
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => {
                temp_file = Some(file);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let mut temp_file = temp_file.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique temporary dotenv file",
        )
    })?;

    let write_result = (|| {
        temp_file.write_all(contents)?;
        temp_file.flush()?;
        temp_file.sync_all()?;
        drop(temp_file);
        fs::rename(&temp_path, path)
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

fn escape_dotenv_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Save only edited entries back to the source they were loaded from.
pub fn save_changes(
    path: impl AsRef<Path>,
    original: &[EnvEntry],
    current: &[EnvEntry],
) -> io::Result<()> {
    let entries_are_file_backed = original
        .iter()
        .chain(current)
        .all(|entry| matches!(entry.source, EnvSource::Dotenv | EnvSource::Process));
    if entries_are_file_backed {
        if path.as_ref().exists() {
            return save_dotenv_changes(path, current);
        }
        return save_env(path, current);
    }

    #[cfg(windows)]
    {
        if !has_admin_privileges() && system_variables_changed(original, current) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "administrator rights are required to save System environment variables; restart envui as administrator",
            ));
        }
        save_windows_changes(original, current)
    }
    #[cfg(not(windows))]
    {
        let _ = (original, current);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows registry storage is only available on Windows",
        ))
    }
}

#[cfg(windows)]
fn system_variables_changed(original: &[EnvEntry], current: &[EnvEntry]) -> bool {
    original
        .iter()
        .filter(|entry| entry.source == EnvSource::System)
        .any(|old| {
            current
                .iter()
                .find(|entry| entry.source == EnvSource::System && entry.key == old.key)
                .is_none_or(|entry| entry.value != old.value || entry.expandable != old.expandable)
        })
        || current
            .iter()
            .filter(|entry| entry.source == EnvSource::System)
            .any(|entry| {
                !original
                    .iter()
                    .any(|old| old.source == EnvSource::System && old.key == entry.key)
            })
}

#[cfg(windows)]
fn load_windows_environment() -> io::Result<Vec<EnvEntry>> {
    use winreg::{RegKey, enums::*};

    let mut entries = Vec::new();
    read_registry_scope(
        &RegKey::predef(HKEY_CURRENT_USER),
        "Environment",
        EnvSource::User,
        &mut entries,
    )?;
    read_registry_scope(
        &RegKey::predef(HKEY_LOCAL_MACHINE),
        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        EnvSource::System,
        &mut entries,
    )?;
    entries.sort_by(|left, right| {
        source_order(left.source)
            .cmp(&source_order(right.source))
            .then_with(|| left.key.to_lowercase().cmp(&right.key.to_lowercase()))
    });
    Ok(entries)
}

#[cfg(windows)]
fn read_registry_scope(
    root: &winreg::RegKey,
    path: &str,
    source: EnvSource,
    entries: &mut Vec<EnvEntry>,
) -> io::Result<()> {
    use winreg::enums::*;

    let key = root.open_subkey_with_flags(path, KEY_READ)?;
    for item in key.enum_values() {
        let (name, raw_value) = item?;
        if raw_value.vtype != REG_SZ && raw_value.vtype != REG_EXPAND_SZ {
            continue;
        }
        let value = key.get_value::<String, _>(&name)?;
        entries.push(EnvEntry {
            key: name,
            value,
            source,
            expandable: raw_value.vtype == REG_EXPAND_SZ,
            leading_lines: Vec::new(),
            trailing_lines: Vec::new(),
        });
    }
    Ok(())
}

#[cfg(windows)]
fn source_order(source: EnvSource) -> u8 {
    match source {
        EnvSource::User => 0,
        EnvSource::System => 1,
        EnvSource::Dotenv => 2,
        EnvSource::Process => 3,
    }
}

#[cfg(windows)]
fn save_windows_changes(original: &[EnvEntry], current: &[EnvEntry]) -> io::Result<()> {
    use winreg::{RegKey, enums::*, types::ToRegValue};

    let mut changed_any = false;
    for entry in current {
        let previous = original
            .iter()
            .find(|old| old.key == entry.key && old.source == entry.source);
        if previous.is_some_and(|old| old.value == entry.value) {
            continue;
        }

        let (root, path) = match entry.source {
            EnvSource::User => (RegKey::predef(HKEY_CURRENT_USER), "Environment"),
            EnvSource::System => (
                RegKey::predef(HKEY_LOCAL_MACHINE),
                "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
            ),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cannot save a non-Windows environment entry to the registry",
                ));
            }
        };
        let key = root.open_subkey_with_flags(path, KEY_SET_VALUE)?;
        let mut raw_value = entry.value.to_reg_value();
        if entry.expandable {
            raw_value.vtype = REG_EXPAND_SZ;
        }
        key.set_raw_value(&entry.key, &raw_value)?;
        changed_any = true;
    }

    for entry in original {
        let still_exists = current
            .iter()
            .any(|current| current.key == entry.key && current.source == entry.source);
        if still_exists {
            continue;
        }
        let (root, path) = match entry.source {
            EnvSource::User => (RegKey::predef(HKEY_CURRENT_USER), "Environment"),
            EnvSource::System => (
                RegKey::predef(HKEY_LOCAL_MACHINE),
                "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
            ),
            _ => continue,
        };
        let key = root.open_subkey_with_flags(path, KEY_SET_VALUE)?;
        key.delete_value(&entry.key)?;
        changed_any = true;
    }

    if changed_any {
        broadcast_environment_change();
    }
    Ok(())
}

#[cfg(windows)]
fn broadcast_environment_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };

    let environment: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result = 0;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            environment.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}
