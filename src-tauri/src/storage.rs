use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temporary_path(destination: &Path) -> Result<PathBuf, String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "Destination has no parent directory".to_string())?;
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Destination has no valid file name".to_string())?;
    let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);

    Ok(parent.join(format!(
        ".{name}.split-tmp-{}-{sequence}",
        std::process::id()
    )))
}

fn wide_null(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub fn atomic_write(destination: &Path, content: impl AsRef<[u8]>) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "Destination has no parent directory".to_string())?;

    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create destination directory: {error}"))?;

    let temporary = temporary_path(destination)?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("Could not create temporary file: {error}"))?;

        file.write_all(content.as_ref())
            .map_err(|error| format!("Could not write temporary file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Could not flush temporary file: {error}"))?;
        drop(file);

        let temporary_wide = wide_null(&temporary);
        let destination_wide = wide_null(destination);
        let moved = unsafe {
            MoveFileExW(
                temporary_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };

        if moved == 0 {
            return Err(format!(
                "Could not replace destination file: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomically_creates_and_replaces_a_file() {
        let directory = std::env::temp_dir().join(format!(
            "split-atomic-write-test-{}-{}",
            std::process::id(),
            TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).expect("test directory should be created");
        let destination = directory.join("state.json");

        atomic_write(&destination, b"first").expect("initial write should succeed");
        atomic_write(&destination, b"second").expect("replacement should succeed");

        assert_eq!(fs::read(&destination).unwrap(), b"second");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);

        fs::remove_dir_all(directory).expect("test directory should be removed");
    }
}
