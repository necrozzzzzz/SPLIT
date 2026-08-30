use std::{
    ffi::OsString,
    fs,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::{
            Debug::{ReadProcessMemory, WriteProcessMemory},
            ToolHelp::{
                CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W,
                TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
            },
        },
        Threading::{
            OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ,
            PROCESS_VM_WRITE,
        },
    },
};

use super::process;

const CLIENT_MODULE: &str = "client.dll";

const CAMERA_RTTI_NAME: &[u8] = b".?AVCCitadel_ThirdPersonCamera@@";

/*
 * CBaseCamera / CCitadel_ThirdPersonCamera
 *
 * Validé sur la build actuelle :
 *
 * +0x44 pitch
 * +0x48 yaw
 * +0x4C roll
 */
const CAMERA_ANGLES_OFFSET: usize = 0x44;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraSnapshot {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CameraAngles {
    pitch: f32,
    yaw: f32,
    roll: f32,
}

#[derive(Debug, Clone, Copy)]
struct CameraRuntime {
    pid: u32,

    client_base: usize,

    camera_global: usize,

    primary_vtable: usize,
}

const CAMERA_CACHE_FILE: &str = "camera-runtime-cache.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CameraRuntimeCache {
    client_size: usize,
    pe_timestamp: u32,

    camera_global_rva: usize,
    primary_vtable_rva: usize,
}

static CAMERA_RUNTIME: Mutex<Option<CameraRuntime>> = Mutex::new(None);

fn camera_cache_path() -> Result<PathBuf, String> {
    let app_data =
        std::env::var_os("APPDATA").ok_or_else(|| "APPDATA is unavailable".to_string())?;

    Ok(PathBuf::from(app_data)
        .join("SPLIT")
        .join(CAMERA_CACHE_FILE))
}

fn read_pe_timestamp(process: HANDLE, client_base: usize) -> Result<u32, String> {
    /*
     * IMAGE_DOS_HEADER.e_lfanew
     */
    let pe_offset = read_value::<u32>(process, client_base + 0x3C)? as usize;

    /*
     * e_lfanew doit rester dans une zone
     * raisonnable du début du module.
     */
    if pe_offset < 0x40 || pe_offset > 0x4000 {
        return Err(format!("Suspicious PE header offset: 0x{pe_offset:X}"));
    }

    let pe_header = client_base + pe_offset;

    /*
     * "PE\0\0"
     */
    let signature = read_value::<u32>(process, pe_header)?;

    if signature != 0x0000_4550 {
        return Err(format!(
            "Invalid client.dll PE signature: 0x{signature:08X}"
        ));
    }

    /*
     * IMAGE_FILE_HEADER.TimeDateStamp
     *
     * PE signature : +0x00
     * Machine      : +0x04
     * Sections     : +0x06
     * TimeDateStamp: +0x08
     */
    read_value::<u32>(process, pe_header + 0x08)
}

fn load_persistent_runtime(
    process: HANDLE,
    pid: u32,
    client_base: usize,
    client_size: usize,
    pe_timestamp: u32,
) -> Option<CameraRuntime> {
    let path = camera_cache_path().ok()?;

    let raw = fs::read_to_string(path).ok()?;

    let cache = serde_json::from_str::<CameraRuntimeCache>(&raw).ok()?;

    /*
     * Deadlock a changé :
     * on rejette immédiatement le cache.
     */
    if cache.client_size != client_size || cache.pe_timestamp != pe_timestamp {
        println!("[SPLIT] Camera cache stale");

        return None;
    }

    if cache.camera_global_rva >= client_size || cache.primary_vtable_rva >= client_size {
        println!("[SPLIT] Camera cache contains invalid RVA");

        return None;
    }

    let camera_global = client_base + cache.camera_global_rva;

    let primary_vtable = client_base + cache.primary_vtable_rva;

    /*
     * Validation runtime réelle :
     *
     * global -> camera object -> vtable
     *
     * Si ça ne correspond plus,
     * aucun offset caché n'est utilisé.
     */
    let camera_object = read_value::<usize>(process, camera_global).ok()?;

    if camera_object < 0x10000 {
        return None;
    }

    let actual_vtable = read_value::<usize>(process, camera_object).ok()?;

    if actual_vtable != primary_vtable {
        println!("[SPLIT] Camera cache runtime validation failed");

        return None;
    }

    println!("[SPLIT] Camera cache HIT");

    Some(CameraRuntime {
        pid,
        client_base,
        camera_global,
        primary_vtable,
    })
}

fn save_persistent_runtime(runtime: CameraRuntime, client_size: usize, pe_timestamp: u32) {
    let Ok(path) = camera_cache_path() else {
        return;
    };

    let Some(parent) = path.parent() else {
        return;
    };

    if let Err(error) = fs::create_dir_all(parent) {
        eprintln!("[SPLIT] Could not create camera cache directory: {error}");

        return;
    }

    let cache = CameraRuntimeCache {
        client_size,
        pe_timestamp,

        camera_global_rva: runtime.camera_global - runtime.client_base,

        primary_vtable_rva: runtime.primary_vtable - runtime.client_base,
    };

    let Ok(json) = serde_json::to_string_pretty(&cache) else {
        return;
    };

    if let Err(error) = fs::write(&path, json) {
        eprintln!("[SPLIT] Could not write camera cache: {error}");

        return;
    }

    println!("[SPLIT] Camera cache saved");
}

fn wide_to_string(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(value.len());

    OsString::from_wide(&value[..length])
        .to_string_lossy()
        .into_owned()
}

fn find_module(pid: u32, module_name: &str) -> Result<(usize, usize), String> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);

        if snapshot == INVALID_HANDLE_VALUE {
            return Err(format!(
                "Could not enumerate Deadlock modules: {}",
                std::io::Error::last_os_error(),
            ));
        }

        let mut entry: MODULEENTRY32W = zeroed();

        entry.dwSize = size_of::<MODULEENTRY32W>() as u32;

        let mut result = None;

        if Module32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name = wide_to_string(&entry.szModule);

                if name.eq_ignore_ascii_case(module_name) {
                    result = Some((entry.modBaseAddr as usize, entry.modBaseSize as usize));

                    break;
                }

                if Module32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);

        result.ok_or_else(|| format!("{module_name} was not found in Deadlock"))
    }
}

fn open_deadlock(pid: u32) -> Result<HANDLE, String> {
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION,
            0,
            pid,
        )
    };

    if process.is_null() {
        return Err(format!(
            "Could not open Deadlock: {}",
            std::io::Error::last_os_error(),
        ));
    }

    Ok(process)
}

fn read_bytes(process: HANDLE, address: usize, size: usize) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0_u8; size];

    let mut read = 0_usize;

    let success = unsafe {
        ReadProcessMemory(
            process,
            address as *const _,
            buffer.as_mut_ptr() as *mut _,
            size,
            &mut read,
        )
    };

    if success == 0 {
        return Err(format!(
            "Could not read Deadlock memory at 0x{address:X}: {}",
            std::io::Error::last_os_error(),
        ));
    }

    buffer.truncate(read);

    Ok(buffer)
}

fn read_value<T: Copy>(process: HANDLE, address: usize) -> Result<T, String> {
    let mut value = std::mem::MaybeUninit::<T>::uninit();

    let mut read = 0_usize;

    let success = unsafe {
        ReadProcessMemory(
            process,
            address as *const _,
            value.as_mut_ptr() as *mut _,
            size_of::<T>(),
            &mut read,
        )
    };

    if success == 0 || read != size_of::<T>() {
        return Err(format!(
            "Could not read {} bytes at 0x{address:X}",
            size_of::<T>(),
        ));
    }

    Ok(unsafe { value.assume_init() })
}

fn write_value<T: Copy>(process: HANDLE, address: usize, value: &T) -> Result<(), String> {
    let mut written = 0_usize;

    let success = unsafe {
        WriteProcessMemory(
            process,
            address as *mut _,
            value as *const T as *const _,
            size_of::<T>(),
            &mut written,
        )
    };

    if success == 0 || written != size_of::<T>() {
        return Err(format!(
            "Could not write {} bytes at 0x{address:X}: {}",
            size_of::<T>(),
            std::io::Error::last_os_error(),
        ));
    }

    Ok(())
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    if offset + 4 > data.len() {
        return None;
    }

    Some(u32::from_le_bytes(
        data[offset..offset + 4].try_into().ok()?,
    ))
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    if offset + 8 > data.len() {
        return None;
    }

    Some(u64::from_le_bytes(
        data[offset..offset + 8].try_into().ok()?,
    ))
}

fn find_all_bytes(data: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > data.len() {
        return Vec::new();
    }

    data.windows(needle.len())
        .enumerate()
        .filter_map(
            |(offset, window)| {
                if window == needle {
                    Some(offset)
                } else {
                    None
                }
            },
        )
        .collect()
}

fn find_absolute_pointer_refs(module: &[u8], target: usize) -> Vec<usize> {
    let needle = (target as u64).to_le_bytes();

    module
        .windows(8)
        .enumerate()
        .filter_map(|(offset, window)| {
            if window == needle.as_slice() {
                Some(offset)
            } else {
                None
            }
        })
        .collect()
}

fn rip_target(module: &[u8], module_base: usize, offset: usize) -> Option<usize> {
    if offset + 7 > module.len() {
        return None;
    }

    let rex = module[offset];

    if !(0x40..=0x4F).contains(&rex) {
        return None;
    }

    let opcode = module[offset + 1];

    if opcode != 0x8D && opcode != 0x8B {
        return None;
    }

    let modrm = module[offset + 2];

    /*
     * mod = 00
     * r/m = 101
     *
     * => RIP-relative.
     */
    if modrm & 0xC7 != 0x05 {
        return None;
    }

    let displacement = i32::from_le_bytes([
        module[offset + 3],
        module[offset + 4],
        module[offset + 5],
        module[offset + 6],
    ]);

    Some((module_base + offset + 7).wrapping_add_signed(displacement as isize))
}

fn find_rip_xrefs(module: &[u8], module_base: usize, target: usize) -> Vec<usize> {
    if module.len() < 7 {
        return Vec::new();
    }

    let mut results = Vec::new();

    for offset in 0..=module.len() - 7 {
        if rip_target(module, module_base, offset) == Some(target) {
            results.push(offset);
        }
    }

    results
}

fn resolve_camera_rtti(module: &[u8], module_base: usize) -> Result<(usize, usize), String> {
    let occurrences = find_all_bytes(module, CAMERA_RTTI_NAME);

    let rtti_name = *occurrences
        .first()
        .ok_or_else(|| "CCitadel_ThirdPersonCamera RTTI was not found".to_string())?;

    if rtti_name < 0x10 {
        return Err("Invalid camera RTTI TypeDescriptor".to_string());
    }

    /*
     * MSVC x64 TypeDescriptor :
     *
     * +0x00 vftable
     * +0x08 spare
     * +0x10 ".?AVClass@@"
     */
    let type_descriptor = rtti_name - 0x10;

    let type_descriptor_rva = type_descriptor as u32;

    let needle = type_descriptor_rva.to_le_bytes();

    let mut best_vtable: Option<(usize, usize)> = None;

    for (type_ref, window) in module.windows(4).enumerate() {
        if window != needle.as_slice() {
            continue;
        }

        if type_ref < 0x0C {
            continue;
        }

        /*
         * _RTTICompleteObjectLocator
         *
         * +0x0C pTypeDescriptor RVA
         * +0x14 pSelf RVA
         */
        let col = type_ref - 0x0C;

        let Some(signature) = read_u32(module, col) else {
            continue;
        };

        let Some(p_type) = read_u32(module, col + 0x0C) else {
            continue;
        };

        let Some(p_self) = read_u32(module, col + 0x14) else {
            continue;
        };

        if signature != 0 && signature != 1 {
            continue;
        }

        if p_type != type_descriptor_rva {
            continue;
        }

        if p_self != col as u32 {
            continue;
        }

        let col_address = module_base + col;

        for col_ref in find_absolute_pointer_refs(module, col_address) {
            /*
             * vtable[-1] = COL
             * vtable[0]  = virtual fn 0
             */
            let vtable_offset = col_ref + 8;

            if vtable_offset + 80 > module.len() {
                continue;
            }

            /*
             * CCitadel_ThirdPersonCamera a
             * une grosse vtable principale.
             *
             * La vtable secondaire trouvée
             * pendant le probe n'avait que
             * quelques pointeurs de code.
             */
            let mut score = 0_usize;

            for index in 0..10 {
                let Some(function) = read_u64(module, vtable_offset + index * 8) else {
                    continue;
                };

                let function = function as usize;

                if function >= module_base && function < module_base + module.len() {
                    score += 1;
                }
            }

            if best_vtable.is_none_or(|(_, best_score)| score > best_score) {
                best_vtable = Some((module_base + vtable_offset, score));
            }
        }
    }

    let Some((vtable, score)) = best_vtable else {
        return Err("Could not resolve CCitadel_ThirdPersonCamera vtable".to_string());
    };

    if score < 6 {
        return Err(format!(
            "Camera RTTI vtable confidence is too low ({score}/10)"
        ));
    }

    Ok((module_base + type_descriptor, vtable))
}

fn resolve_camera_global(
    process: HANDLE,
    module: &[u8],
    module_base: usize,
    type_descriptor: usize,
    primary_vtable: usize,
) -> Result<usize, String> {
    let xrefs = find_rip_xrefs(module, module_base, type_descriptor);

    /*
     * Dans la build actuelle, les fonctions
     * qui référencent le TypeDescriptor
     * CCitadel_ThirdPersonCamera ont aussi,
     * quelques instructions auparavant,
     * un LEA vers le slot global contenant
     * CCitadel_ThirdPersonCamera*.
     *
     * On ne dépend pas de son RVA exact :
     * on valide chaque candidat grâce
     * à la vtable RTTI.
     */
    for xref in xrefs {
        let start = xref.saturating_sub(0x60);

        let end = (xref + 0x30).min(module.len().saturating_sub(7));

        for offset in start..=end {
            let Some(target) = rip_target(module, module_base, offset) else {
                continue;
            };

            if target < module_base || target >= module_base + module.len() {
                continue;
            }

            let Ok(camera_object) = read_value::<usize>(process, target) else {
                continue;
            };

            if camera_object == 0 {
                continue;
            }

            let Ok(object_vtable) = read_value::<usize>(process, camera_object) else {
                continue;
            };

            if object_vtable == primary_vtable {
                return Ok(target);
            }
        }
    }

    Err("Could not resolve active CCitadel_ThirdPersonCamera global".to_string())
}

fn resolve_runtime(pid: u32) -> Result<CameraRuntime, String> {
    let (client_base, client_size) = find_module(pid, CLIENT_MODULE)?;

    let process = open_deadlock(pid)?;

    let result = (|| {
        let pe_timestamp = read_pe_timestamp(process, client_base)?;

        /*
         * CHEMIN RAPIDE.
         *
         * Aucun dump de 52 Mo,
         * aucune recherche RTTI.
         */
        if let Some(runtime) =
            load_persistent_runtime(process, pid, client_base, client_size, pe_timestamp)
        {
            println!("[SPLIT] Camera resolved from persistent cache");

            return Ok(runtime);
        }

        /*
         * CHEMIN LENT.
         *
         * Uniquement si :
         * - aucun cache
         * - update Deadlock
         * - cache invalide
         */
        println!("[SPLIT] Camera cache MISS -> full RTTI scan");

        println!("[SPLIT] Resolving Deadlock camera...");

        let module = read_bytes(process, client_base, client_size)?;

        let (type_descriptor, primary_vtable) = resolve_camera_rtti(&module, client_base)?;

        let camera_global = resolve_camera_global(
            process,
            &module,
            client_base,
            type_descriptor,
            primary_vtable,
        )?;

        println!("[SPLIT] Camera resolved:");

        println!(
            "[SPLIT]   vtable = client.dll+0x{:X}",
            primary_vtable - client_base,
        );

        println!(
            "[SPLIT]   global = client.dll+0x{:X}",
            camera_global - client_base,
        );

        let runtime = CameraRuntime {
            pid,
            client_base,
            camera_global,
            primary_vtable,
        };

        save_persistent_runtime(runtime, client_size, pe_timestamp);

        Ok(runtime)
    })();

    let _ = unsafe { CloseHandle(process) };

    result
}

fn runtime_for_pid(pid: u32) -> Result<CameraRuntime, String> {
    if let Ok(cache) = CAMERA_RUNTIME.lock() {
        if let Some(runtime) = *cache {
            if runtime.pid == pid {
                return Ok(runtime);
            }
        }
    }

    let runtime = resolve_runtime(pid)?;

    let mut cache = CAMERA_RUNTIME
        .lock()
        .map_err(|_| "Camera runtime lock poisoned".to_string())?;

    *cache = Some(runtime);

    Ok(runtime)
}

fn invalidate_runtime(pid: u32) {
    if let Ok(mut cache) = CAMERA_RUNTIME.lock() {
        if cache.as_ref().is_some_and(|runtime| runtime.pid == pid) {
            *cache = None;
        }
    }
}

fn active_camera_object(process: HANDLE, runtime: CameraRuntime) -> Result<usize, String> {
    let camera_object = read_value::<usize>(process, runtime.camera_global)?;

    if camera_object == 0 {
        return Err("Deadlock camera object is NULL".to_string());
    }

    let object_vtable = read_value::<usize>(process, camera_object)?;

    if object_vtable != runtime.primary_vtable {
        return Err(format!(
            "Deadlock camera vtable changed: expected 0x{:X}, got 0x{object_vtable:X}",
            runtime.primary_vtable,
        ));
    }

    Ok(camera_object)
}

pub fn capture() -> Result<CameraSnapshot, String> {
    let pid = process::deadlock_pid().ok_or_else(|| "Deadlock is not running".to_string())?;

    let runtime = runtime_for_pid(pid)?;

    let process = open_deadlock(pid)?;

    let result = (|| {
        let camera_object = active_camera_object(process, runtime)?;

        let angles: CameraAngles = read_value(process, camera_object + CAMERA_ANGLES_OFFSET)?;

        Ok(CameraSnapshot {
            pitch: angles.pitch,
            yaw: angles.yaw,
            roll: angles.roll,
        })
    })();

    let _ = unsafe { CloseHandle(process) };

    if result.is_err() {
        invalidate_runtime(pid);
    }

    result
}

pub fn restore(snapshot: CameraSnapshot) -> Result<(), String> {
    let pid = process::deadlock_pid().ok_or_else(|| "Deadlock is not running".to_string())?;

    let runtime = runtime_for_pid(pid)?;

    let process = open_deadlock(pid)?;

    let result = (|| {
        let camera_object = active_camera_object(process, runtime)?;

        let angles = CameraAngles {
            pitch: snapshot.pitch,
            yaw: snapshot.yaw,
            roll: snapshot.roll,
        };

        write_value(process, camera_object + CAMERA_ANGLES_OFFSET, &angles)
    })();

    let _ = unsafe { CloseHandle(process) };

    if result.is_err() {
        invalidate_runtime(pid);
    }

    result
}
