use std::{
    ffi::OsString,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStringExt,
    path::PathBuf,
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};

const DEADLOCK_PROCESS_NAME: &str = "deadlock.exe";

fn wide_to_string(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(value.len());

    String::from_utf16_lossy(&value[..length])
}

pub(crate) fn deadlock_pid() -> Option<u32> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);

        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }

        let mut entry: PROCESSENTRY32W = zeroed();

        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;

        let mut result = None;

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name = wide_to_string(&entry.szExeFile);

                if name.eq_ignore_ascii_case(DEADLOCK_PROCESS_NAME) {
                    result = Some(entry.th32ProcessID);

                    break;
                }

                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);

        result
    }
}

pub fn is_deadlock_running() -> bool {
    deadlock_pid().is_some()
}

fn process_exe_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);

        if process.is_null() {
            return None;
        }

        /*
         * QueryFullProcessImageNameW
         * accepte une taille en caractères.
         *
         * 32768 couvre largement
         * les chemins Windows étendus.
         */
        let mut buffer = vec![0u16; 32768];

        let mut length = buffer.len() as u32;

        let success = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length);

        let _ = CloseHandle(process);

        if success == 0 || length == 0 {
            return None;
        }

        buffer.truncate(length as usize);

        Some(PathBuf::from(OsString::from_wide(&buffer)))
    }
}

pub fn running_deadlock_root() -> Option<PathBuf> {
    let pid = deadlock_pid()?;

    let executable = process_exe_path(pid)?;

    /*
     * Deadlock
     * └── game
     *     └── bin
     *         └── win64
     *             └── deadlock.exe
     */
    let root = executable.parent()?.parent()?.parent()?.parent()?;

    let expected_executable = root
        .join("game")
        .join("bin")
        .join("win64")
        .join("deadlock.exe");

    if expected_executable.is_file() {
        Some(root.to_path_buf())
    } else {
        None
    }
}
