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
            CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW,
            Process32NextW, MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE,
            TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
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

fn process_has_module(pid: u32, module_name: &str) -> bool {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);

        /*
         * Certains processus portant le même
         * nom peuvent être inaccessibles.
         *
         * Ce n'est pas une erreur fatale :
         * ce candidat n'est simplement pas
         * le process de jeu que l'on cherche.
         */
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }

        let mut entry: MODULEENTRY32W = zeroed();

        entry.dwSize = size_of::<MODULEENTRY32W>() as u32;

        let mut found = false;

        if Module32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name = wide_to_string(&entry.szModule);

                if name.eq_ignore_ascii_case(module_name) {
                    found = true;
                    break;
                }

                if Module32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);

        found
    }
}

pub(crate) fn deadlock_pid() -> Option<u32> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);

        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }

        let mut entry: PROCESSENTRY32W = zeroed();

        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;

        /*
         * fallback :
         *
         * si Windows refuse l'énumération
         * des modules, SPLIT doit quand même
         * pouvoir détecter Deadlock pour les
         * fonctionnalités qui n'ont pas besoin
         * d'accéder à sa mémoire.
         */
        let mut fallback = None;

        let mut result = None;

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name = wide_to_string(&entry.szExeFile);

                if name.eq_ignore_ascii_case(DEADLOCK_PROCESS_NAME) {
                    let pid = entry.th32ProcessID;

                    /*
                     * On privilégie un processus
                     * dont le vrai chemin exécutable
                     * est accessible.
                     */
                    if process_exe_path(pid).is_some() {
                        fallback.get_or_insert(pid);

                        /*
                         * client.dll est la preuve
                         * que ce processus est bien
                         * l'instance de jeu Deadlock
                         * dont SPLIT a besoin.
                         *
                         * Un autre deadlock.exe
                         * inaccessible ou secondaire
                         * est donc ignoré.
                         */
                        if process_has_module(pid, "client.dll") {
                            result = Some(pid);

                            break;
                        }
                    }
                }

                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);

        /*
         * Priorité :
         *
         * 1. deadlock.exe + client.dll
         * 2. deadlock.exe avec chemin valide
         *
         * Le fallback conserve la détection
         * générale même si l'accès aux modules
         * est bloqué par Windows.
         */
        result.or(fallback)
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
