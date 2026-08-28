use std::{
    ffi::OsString,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStringExt,
    thread,
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::{
            Debug::{ReadProcessMemory, WriteProcessMemory},
            ToolHelp::{
                CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW,
                Process32NextW, MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE,
                TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
            },
        },
        Memory::{
            VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_PRIVATE, PAGE_GUARD,
            PAGE_NOACCESS,
        },
        Threading::{
            OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ,
            PROCESS_VM_WRITE,
        },
    },
    UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F6, VK_F7, VK_F8, VK_F9},
};

const PROCESS_NAME: &str = "deadlock.exe";

/*
 * Pattern observé dans client.dll pour
 * obtenir l'objet prediction.
 *
 * On le considère comme EXPÉRIMENTAL :
 * si Valve l'a modifié, le probe nous
 * le dira proprement.
 */
const PREDICTION_PATTERN: &str = "48 8D 05 ?? ?? ?? ?? C3 CC CC CC CC CC CC CC CC 40 53 56 41 54";

/*
 * Recherche Deadlock 2026 :
 *
 * prediction + 0xD8
 *   -> C_CitadelPlayerPawn*
 */
const LOCAL_PAWN_FROM_PREDICTION: usize = 0xD8;

/*
 * Dump 2026 :
 *
 * C_CitadelPlayerPawn
 *   + 0x1240
 *   -> QAngle m_angClientCamera
 *
 * À valider sur TA build actuelle.
 */
/*
 * Confirmé :
 * C_CitadelPlayerPawn::m_angClientCamera
 */
const ANG_CLIENT_CAMERA: usize = 0x1248;

const ANG_EYE_ANGLES: usize = 0x11B8;

const SCENE_NODE_OWNER_OFFSET: usize = 0x30;
const SCENE_NODE_LOCAL_ROTATION: usize = 0xB8;
const SCENE_NODE_ABS_ROTATION: usize = 0xD4;

const CAMERA_POSITION_OFFSET: usize = 0x38;
const CAMERA_ANGLES_OFFSET: usize = 0x44;

const MOVEMENT_SERVICES_OFFSET: usize = 0xF28;
const OLD_VIEW_ANGLES_OFFSET: usize = 0x228;

const ANIM_GRAPH_UPDATE_ENABLED: usize = 0xA70;

/*
 * CCitadel_ThirdPersonCamera*
 * Build Deadlock actuelle.
 *
 * Trouvé dynamiquement :
 * client.dll + 0x32B13F8
 */
const THIRD_PERSON_CAMERA_GLOBAL: usize = 0x32B13F8;

const THIRD_PERSON_CAMERA_PRIMARY_VTABLE: usize = 0x233D410;

const V_ANGLE: usize = 0xFA0;

const V_ANGLE_PREVIOUS: usize = 0xFAC;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct QAngle {
    pitch: f32,
    yaw: f32,
    roll: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
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

fn find_process_id(process_name: &str) -> Result<u32, String> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);

        if snapshot == INVALID_HANDLE_VALUE {
            return Err(format!(
                "CreateToolhelp32Snapshot(process) failed: {}",
                std::io::Error::last_os_error(),
            ));
        }

        let mut entry: PROCESSENTRY32W = zeroed();

        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;

        let mut found = None;

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name = wide_to_string(&entry.szExeFile);

                if name.eq_ignore_ascii_case(process_name) {
                    found = Some(entry.th32ProcessID);

                    break;
                }

                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        CloseHandle(snapshot);

        found.ok_or_else(|| format!("{process_name} is not running"))
    }
}

fn find_module(pid: u32, module_name: &str) -> Result<(usize, usize), String> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);

        if snapshot == INVALID_HANDLE_VALUE {
            return Err(format!(
                "CreateToolhelp32Snapshot(module) failed: {}",
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

        CloseHandle(snapshot);

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
            "OpenProcess failed: {}",
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
            "ReadProcessMemory failed at 0x{address:X}: {}",
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
            "Could not read {} bytes at 0x{address:X}: {}",
            size_of::<T>(),
            std::io::Error::last_os_error(),
        ));
    }

    Ok(unsafe { value.assume_init() })
}

fn scan_private_memory_for_pointer(process: HANDLE, target: usize) -> Result<Vec<usize>, String> {
    let mut results = Vec::<usize>::new();

    let target_bytes = (target as u64).to_le_bytes();

    let mut address = 0_usize;

    const CHUNK_SIZE: usize = 1024 * 1024;

    println!("[probe] Scanning private committed memory for 0x{target:X}...");

    loop {
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };

        let queried = unsafe {
            VirtualQueryEx(
                process,
                address as *const _,
                &mut info,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };

        if queried == 0 {
            break;
        }

        let base = info.BaseAddress as usize;

        let region_size = info.RegionSize;

        let next = match base.checked_add(region_size) {
            Some(value) => value,
            None => break,
        };

        let committed = info.State == MEM_COMMIT;

        let private = info.Type == MEM_PRIVATE;

        let guarded = (info.Protect & PAGE_GUARD) != 0;

        let no_access = (info.Protect & PAGE_NOACCESS) != 0;

        if committed && private && !guarded && !no_access && region_size >= 8 {
            let mut chunk_offset = 0_usize;

            while chunk_offset < region_size {
                let remaining = region_size - chunk_offset;

                let size = remaining.min(CHUNK_SIZE);

                let chunk_address = base + chunk_offset;

                if let Ok(bytes) = read_bytes(process, chunk_address, size) {
                    if bytes.len() >= 8 {
                        /*
                         * Les objets/vptr x64
                         * sont normalement
                         * alignés sur 8 octets.
                         */
                        for offset in (0..=bytes.len() - 8).step_by(8) {
                            if bytes[offset..offset + 8] == target_bytes {
                                results.push(chunk_address + offset);

                                if results.len() >= 128 {
                                    return Ok(results);
                                }
                            }
                        }
                    }
                }

                chunk_offset = match chunk_offset.checked_add(size) {
                    Some(value) => value,
                    None => break,
                };
            }
        }

        if next <= address {
            break;
        }

        address = next;
    }

    Ok(results)
}

fn scan_client_for_camera_object_pointer(
    process: HANDLE,
    client_base: usize,
    client_size: usize,
    camera_object: usize,
) -> Result<(), String> {
    println!();
    println!("=== CAMERA GLOBAL POINTER SCAN ===");

    println!("[probe] Known live camera object = 0x{camera_object:X}");

    println!(
        "[probe] Reading client.dll only ({:.1} MB)...",
        client_size as f64 / 1024.0 / 1024.0,
    );

    let module = read_bytes(process, client_base, client_size)?;

    let refs = find_absolute_pointer_refs(&module, camera_object);

    println!(
        "[probe] Absolute refs to camera object in client.dll: {}",
        refs.len(),
    );

    if refs.is_empty() {
        println!("[probe] No direct global pointer found.");

        return Ok(());
    }

    for pointer_ref in refs {
        let absolute = client_base + pointer_ref;

        println!();
        println!("[probe] CAMERA PTRREF:");

        println!("[probe]   client.dll+0x{pointer_ref:X}");

        println!("[probe]   absolute = 0x{absolute:X}");

        dump_pointer_neighborhood(&module, client_base, pointer_ref);

        /*
         * Maintenant on cherche quelles
         * instructions utilisent CE slot
         * global.
         */
        let xrefs = find_rip_xrefs(&module, client_base, absolute);

        println!("[probe] RIP xrefs to this global: {}", xrefs.len(),);

        for xref in xrefs {
            println!(
                "[probe]   XREF client.dll+0x{xref:X} (0x{:X})",
                client_base + xref,
            );

            dump_hex_context(&module, xref);

            dump_registration_targets(&module, client_base, xref);
        }
    }

    println!();
    println!("=== CAMERA GLOBAL POINTER SCAN FINISHED ===");

    Ok(())
}

fn probe_third_person_camera_instances(process: HANDLE, client_base: usize) -> Result<(), String> {
    println!();
    println!("=== THIRD PERSON CAMERA INSTANCE SCAN ===");

    let vtable = client_base + THIRD_PERSON_CAMERA_PRIMARY_VTABLE;

    println!("[probe] Primary vtable:");

    println!(
        "[probe]   client.dll+0x{:X}",
        THIRD_PERSON_CAMERA_PRIMARY_VTABLE,
    );

    println!("[probe]   absolute = 0x{vtable:X}");

    let candidates = scan_private_memory_for_pointer(process, vtable)?;

    println!();
    println!("[probe] Candidate object count: {}", candidates.len(),);

    if candidates.is_empty() {
        println!("[probe] No heap object using this vtable was found.");

        return Ok(());
    }

    /*
     * On ne garde qu'un nombre raisonnable
     * de candidats pour le test live.
     */
    let candidates: Vec<usize> = candidates.into_iter().take(16).collect();

    let mut previous = Vec::<Option<QAngle>>::new();

    println!();

    for (index, candidate) in candidates.iter().enumerate() {
        println!("[probe] CANDIDATE #{index}: 0x{candidate:X}");

        let angles = read_value::<QAngle>(process, candidate + 0x44).ok();

        match angles {
            Some(value) => {
                println!(
                    "        +0x44 -> P={:.3} Y={:.3} R={:.3}",
                    value.pitch, value.yaw, value.roll,
                );

                previous.push(Some(value));
            }

            None => {
                println!("        +0x44 -> unreadable");

                previous.push(None);
            }
        }

        if let Ok(second) = read_value::<QAngle>(process, candidate + 0xC8) {
            println!(
                "        +0xC8 -> P={:.3} Y={:.3} R={:.3}",
                second.pitch, second.yaw, second.roll,
            );
        }
    }

    println!();

    let camera_object = candidates[0];

    let camera_angles_address = camera_object + 0x44;

    println!("=== FINAL CAMERA WRITE TEST ===");

    println!("[probe] Active camera object = 0x{camera_object:X}");

    println!("[probe] Camera QAngle       = 0x{camera_angles_address:X}");

    println!();
    println!("F6 = sauvegarder la caméra");

    println!("F7 = restaurer la caméra");

    println!("Ctrl+C = quitter");

    println!();

    let mut saved: Option<QAngle> = None;

    let mut f6_was_down = false;

    let mut f7_was_down = false;

    loop {
        let f6_down = (unsafe { GetAsyncKeyState(VK_F6 as i32) } as u16 & 0x8000) != 0;

        let f7_down = (unsafe { GetAsyncKeyState(VK_F7 as i32) } as u16 & 0x8000) != 0;

        if f6_down && !f6_was_down {
            let current: QAngle = read_value(process, camera_angles_address)?;

            saved = Some(current);

            println!(
                "[probe] SAVED FINAL CAMERA -> P={:.3} Y={:.3} R={:.3}",
                current.pitch, current.yaw, current.roll,
            );
        }

        if f7_down && !f7_was_down {
            if let Some(saved_angle) = saved {
                let before: QAngle = read_value(process, camera_angles_address)?;

                println!();
                println!(
                    "[probe] BEFORE WRITE -> P={:.3} Y={:.3} R={:.3}",
                    before.pitch, before.yaw, before.roll,
                );

                println!(
                    "[probe] TARGET       -> P={:.3} Y={:.3} R={:.3}",
                    saved_angle.pitch, saved_angle.yaw, saved_angle.roll,
                );

                write_value(process, camera_angles_address, &saved_angle)?;

                let immediate: QAngle = read_value(process, camera_angles_address)?;

                println!(
                    "[probe] IMMEDIATE    -> P={:.3} Y={:.3} R={:.3}",
                    immediate.pitch, immediate.yaw, immediate.roll,
                );

                thread::sleep(Duration::from_millis(5));

                let after_5: QAngle = read_value(process, camera_angles_address)?;

                println!(
                    "[probe] AFTER 5 ms   -> P={:.3} Y={:.3} R={:.3}",
                    after_5.pitch, after_5.yaw, after_5.roll,
                );

                thread::sleep(Duration::from_millis(15));

                let after_20: QAngle = read_value(process, camera_angles_address)?;

                println!(
                    "[probe] AFTER 20 ms  -> P={:.3} Y={:.3} R={:.3}",
                    after_20.pitch, after_20.yaw, after_20.roll,
                );

                thread::sleep(Duration::from_millis(80));

                let after_100: QAngle = read_value(process, camera_angles_address)?;

                println!(
                    "[probe] AFTER 100 ms -> P={:.3} Y={:.3} R={:.3}",
                    after_100.pitch, after_100.yaw, after_100.roll,
                );

                println!();
            } else {
                println!("[probe] F7 ignored: press F6 first");
            }
        }

        f6_was_down = f6_down;

        f7_was_down = f7_down;

        thread::sleep(Duration::from_millis(2));
    }
}

fn probe_final_camera_object(process: HANDLE, client_base: usize) -> Result<(), String> {
    println!();
    println!("=== FINAL CAMERA DIRECT WRITE TEST ===");

    let global_address = client_base + THIRD_PERSON_CAMERA_GLOBAL;

    println!("[probe] Camera global:");

    println!("[probe]   client.dll+0x{:X}", THIRD_PERSON_CAMERA_GLOBAL,);

    println!("[probe]   absolute = 0x{global_address:X}");

    let camera_object: usize = read_value(process, global_address)?;

    println!("[probe] Camera object = 0x{camera_object:X}");

    if camera_object == 0 {
        return Err("Camera global returned NULL.".to_string());
    }

    /*
     * Vérification de la vtable.
     */
    let object_vtable: usize = read_value(process, camera_object)?;

    let expected_vtable = client_base + THIRD_PERSON_CAMERA_PRIMARY_VTABLE;

    println!("[probe] Object vtable   = 0x{object_vtable:X}");

    println!("[probe] Expected vtable = 0x{expected_vtable:X}");

    if object_vtable != expected_vtable {
        return Err(
            format!(
                "Camera object vtable mismatch: expected 0x{expected_vtable:X}, got 0x{object_vtable:X}"
            ),
        );
    }

    let angles_address = camera_object + 0x44;

    let initial: QAngle = read_value(process, angles_address)?;

    println!();
    println!("[probe] Camera angles address = 0x{angles_address:X}");

    println!(
        "[probe] INITIAL -> P={:.3} Y={:.3} R={:.3}",
        initial.pitch, initial.yaw, initial.roll,
    );

    println!();
    println!("F6 = sauvegarder la caméra");

    println!("F7 = restaurer la caméra");

    println!("Ctrl+C = quitter");

    println!();

    let mut saved: Option<QAngle> = None;

    let mut f6_was_down = false;

    let mut f7_was_down = false;

    loop {
        let f6_down = (unsafe { GetAsyncKeyState(VK_F6 as i32) } as u16 & 0x8000) != 0;

        let f7_down = (unsafe { GetAsyncKeyState(VK_F7 as i32) } as u16 & 0x8000) != 0;

        if f6_down && !f6_was_down {
            let current: QAngle = read_value(process, angles_address)?;

            saved = Some(current);

            println!(
                "[probe] SAVED -> P={:.3} Y={:.3} R={:.3}",
                current.pitch, current.yaw, current.roll,
            );
        }

        if f7_down && !f7_was_down {
            if let Some(target) = saved {
                let before: QAngle = read_value(process, angles_address)?;

                println!();
                println!(
                    "[probe] BEFORE    -> P={:.3} Y={:.3} R={:.3}",
                    before.pitch, before.yaw, before.roll,
                );

                println!(
                    "[probe] TARGET    -> P={:.3} Y={:.3} R={:.3}",
                    target.pitch, target.yaw, target.roll,
                );

                write_value(process, angles_address, &target)?;

                let immediate: QAngle = read_value(process, angles_address)?;

                println!(
                    "[probe] IMMEDIATE -> P={:.3} Y={:.3} R={:.3}",
                    immediate.pitch, immediate.yaw, immediate.roll,
                );

                thread::sleep(Duration::from_millis(5));

                let after_5: QAngle = read_value(process, angles_address)?;

                println!(
                    "[probe] AFTER 5ms -> P={:.3} Y={:.3} R={:.3}",
                    after_5.pitch, after_5.yaw, after_5.roll,
                );

                thread::sleep(Duration::from_millis(15));

                let after_20: QAngle = read_value(process, angles_address)?;

                println!(
                    "[probe] AFTER 20ms -> P={:.3} Y={:.3} R={:.3}",
                    after_20.pitch, after_20.yaw, after_20.roll,
                );

                thread::sleep(Duration::from_millis(80));

                let after_100: QAngle = read_value(process, angles_address)?;

                println!(
                    "[probe] AFTER 100ms -> P={:.3} Y={:.3} R={:.3}",
                    after_100.pitch, after_100.yaw, after_100.roll,
                );

                println!();
            } else {
                println!("[probe] F7 ignored: press F6 first");
            }
        }

        f6_was_down = f6_down;

        f7_was_down = f7_down;

        thread::sleep(Duration::from_millis(2));
    }
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

fn parse_pattern(pattern: &str) -> Result<Vec<Option<u8>>, String> {
    pattern
        .split_whitespace()
        .map(|token| {
            if token == "?" || token == "??" {
                Ok(None)
            } else {
                u8::from_str_radix(token, 16)
                    .map(Some)
                    .map_err(|_| format!("Invalid pattern token: {token}"))
            }
        })
        .collect()
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

fn expand_ascii_string(module: &[u8], occurrence: usize) -> Option<(usize, String)> {
    if occurrence >= module.len() {
        return None;
    }

    /*
     * Remonte jusqu'au début de la
     * chaîne ASCII/NUL.
     */
    let mut start = occurrence;

    let lower_bound = occurrence.saturating_sub(512);

    while start > lower_bound {
        let byte = module[start - 1];

        if byte == 0 {
            break;
        }

        if !(0x20..=0x7E).contains(&byte) {
            break;
        }

        start -= 1;
    }

    /*
     * Puis avance jusqu'au NUL.
     */
    let mut end = occurrence;

    let upper_bound = (occurrence + 512).min(module.len());

    while end < upper_bound {
        let byte = module[end];

        if byte == 0 {
            break;
        }

        if !(0x20..=0x7E).contains(&byte) {
            break;
        }

        end += 1;
    }

    if end <= start {
        return None;
    }

    let text = String::from_utf8_lossy(&module[start..end]).into_owned();

    Some((start, text))
}

fn find_absolute_pointer_refs(module: &[u8], target: usize) -> Vec<usize> {
    if module.len() < 8 {
        return Vec::new();
    }

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

fn dump_pointer_neighborhood(module: &[u8], module_base: usize, pointer_ref: usize) {
    let start = pointer_ref.saturating_sub(24);

    let end = (pointer_ref + 48).min(module.len());

    println!("    pointer table around client.dll+0x{pointer_ref:X}");

    let mut offset = start;

    while offset + 8 <= end {
        let value = u64::from_le_bytes(module[offset..offset + 8].try_into().unwrap()) as usize;

        let marker = if offset == pointer_ref {
            "<-- STRING PTR"
        } else {
            ""
        };

        if value >= module_base && value < module_base + module.len() {
            println!(
                "      +0x{offset:X} : 0x{value:X} -> client.dll+0x{:X} {marker}",
                value - module_base,
            );
        } else {
            println!("      +0x{offset:X} : 0x{value:X} {marker}");
        }

        offset += 8;
    }
}

fn read_u32_le(data: &[u8], offset: usize) -> Option<u32> {
    if offset + 4 > data.len() {
        return None;
    }

    Some(u32::from_le_bytes(
        data[offset..offset + 4].try_into().ok()?,
    ))
}

fn read_u64_le(data: &[u8], offset: usize) -> Option<u64> {
    if offset + 8 > data.len() {
        return None;
    }

    Some(u64::from_le_bytes(
        data[offset..offset + 8].try_into().ok()?,
    ))
}

fn inspect_msvc_rtti_vtable(module: &[u8], module_base: usize, full_start: usize, full_text: &str) {
    /*
     * MSVC x64 TypeDescriptor :
     *
     * +0x00 vftable
     * +0x08 spare
     * +0x10 nom RTTI ".?AV..."
     */
    if full_start < 0x10 {
        return;
    }

    let type_descriptor = full_start - 0x10;

    let type_descriptor_rva = type_descriptor as u32;

    println!();
    println!("================ RTTI VTABLE SCAN ================");

    println!("[probe] RTTI name:");

    println!("[probe]   {full_text}");

    println!("[probe] TypeDescriptor: client.dll+0x{type_descriptor:X}");

    /*
     * Sous MSVC x64, le CompleteObjectLocator
     * référence le TypeDescriptor avec un RVA 32 bits.
     */
    let needle = type_descriptor_rva.to_le_bytes();

    let type_refs: Vec<usize> = module
        .windows(4)
        .enumerate()
        .filter_map(|(offset, window)| {
            if window == needle.as_slice() {
                Some(offset)
            } else {
                None
            }
        })
        .collect();

    println!("[probe] TypeDescriptor RVA refs: {}", type_refs.len(),);

    for type_ref in type_refs {
        /*
         * _RTTICompleteObjectLocator x64 :
         *
         * +0x00 signature
         * +0x04 offset
         * +0x08 cdOffset
         * +0x0C pTypeDescriptor RVA
         * +0x10 pClassDescriptor RVA
         * +0x14 pSelf RVA
         *
         * Comme type_ref correspond à +0x0C,
         * COL = type_ref - 0x0C.
         */
        if type_ref < 0x0C {
            continue;
        }

        let col = type_ref - 0x0C;

        if col + 0x18 > module.len() {
            continue;
        }

        let Some(signature) = read_u32_le(module, col) else {
            continue;
        };

        let Some(p_type) = read_u32_le(module, col + 0x0C) else {
            continue;
        };

        let Some(p_class) = read_u32_le(module, col + 0x10) else {
            continue;
        };

        let Some(p_self) = read_u32_le(module, col + 0x14) else {
            continue;
        };

        /*
         * Validation importante :
         * pTypeDescriptor doit pointer sur
         * notre TypeDescriptor et pSelf doit
         * pointer sur le COL lui-même.
         */
        if p_type != type_descriptor_rva {
            continue;
        }

        if p_self != col as u32 {
            continue;
        }

        if signature != 0 && signature != 1 {
            continue;
        }

        println!();
        println!("[probe] VALID CompleteObjectLocator");

        println!("[probe]   COL    = client.dll+0x{col:X}");

        println!("[probe]   sig    = {signature}");

        println!("[probe]   class  = client.dll+0x{p_class:X}");

        let col_address = module_base + col;

        /*
         * Une vtable MSVC contient normalement :
         *
         * [vtable - 8] -> CompleteObjectLocator
         * [vtable + 0] -> fonction 0
         * [vtable + 8] -> fonction 1
         * ...
         *
         * On cherche donc qui contient
         * l'adresse absolue du COL.
         */
        let col_refs = find_absolute_pointer_refs(module, col_address);

        println!("[probe] COL absolute refs: {}", col_refs.len(),);

        for col_ref in col_refs {
            println!("[probe]   COL PTRREF client.dll+0x{col_ref:X}");

            let vtable = col_ref + 8;

            if vtable >= module.len() {
                continue;
            }

            println!("[probe]   probable VTABLE = client.dll+0x{vtable:X}");

            println!("[probe]   first virtual functions:");

            for index in 0..10 {
                let slot = vtable + index * 8;

                let Some(function_address) = read_u64_le(module, slot) else {
                    break;
                };

                let function_address = function_address as usize;

                if function_address < module_base || function_address >= module_base + module.len()
                {
                    println!("[probe]     [{index}] 0x{function_address:X} (outside client.dll)");

                    continue;
                }

                let function_offset = function_address - module_base;

                println!("[probe]     [{index}] client.dll+0x{function_offset:X}");

                let preview_end = (function_offset + 40).min(module.len());

                print!("                bytes: ");

                for byte in &module[function_offset..preview_end] {
                    print!("{byte:02X} ");
                }

                println!();

                /*
                 * Pour cette _Func_impl_no_alloc,
                 * l'entrée [2] ressemble à :
                 *
                 *   add rcx, 8
                 *   jmp lambda
                 *
                 * C'est très probablement _Do_call.
                 */
                if index == 2
                    && function_offset + 9 <= module.len()
                    && module[function_offset..function_offset + 5]
                        == [0x48, 0x83, 0xC1, 0x08, 0xE9]
                {
                    let relative = i32::from_le_bytes([
                        module[function_offset + 5],
                        module[function_offset + 6],
                        module[function_offset + 7],
                        module[function_offset + 8],
                    ]);

                    let lambda_target =
                        (function_offset + 9).wrapping_add_signed(relative as isize);

                    println!();
                    println!("[probe] >>> probable _Do_call");

                    println!("[probe] >>> lambda target = client.dll+0x{lambda_target:X}");

                    println!(
                        "[probe] >>> absolute      = 0x{:X}",
                        module_base + lambda_target,
                    );

                    dump_hex_context(module, lambda_target);

                    dump_registration_targets(module, module_base, lambda_target);

                    println!("[probe] <<< end lambda analysis");

                    println!();
                }
            }
        }
    }

    println!("====================================================");
    println!();
}

fn scan_third_person_camera_rtti(
    process: HANDLE,
    client_base: usize,
    client_size: usize,
) -> Result<(), String> {
    println!();
    println!("=== CCitadel_ThirdPersonCamera RTTI SCAN ===");

    println!("[probe] Reading client.dll...");

    let module = read_bytes(process, client_base, client_size)?;

    let needle = b".?AVCCitadel_ThirdPersonCamera@@";

    let occurrences = find_all_bytes(&module, needle);

    println!("[probe] RTTI occurrences: {}", occurrences.len(),);

    if occurrences.is_empty() {
        println!("[probe] CCitadel_ThirdPersonCamera RTTI not found.");

        return Ok(());
    }

    for occurrence in occurrences {
        println!();
        println!("[probe] RTTI occurrence: client.dll+0x{occurrence:X}");

        let Some((full_start, full_text)) = expand_ascii_string(&module, occurrence) else {
            println!("[probe] Could not expand RTTI string.");

            continue;
        };

        println!("[probe] FULL RTTI: client.dll+0x{full_start:X}");

        println!("[probe]   \"{full_text}\"");

        inspect_msvc_rtti_vtable(&module, client_base, full_start, &full_text);
    }

    println!("=== THIRD PERSON CAMERA RTTI SCAN FINISHED ===");

    Ok(())
}

fn find_rip_xrefs(module: &[u8], module_base: usize, target: usize) -> Vec<usize> {
    let mut results = Vec::new();

    /*
     * On cherche les formes x64 classiques :
     *
     * 48 8D ?? XX XX XX XX
     * 4C 8D ?? XX XX XX XX
     *
     * = LEA reg, [RIP + rel32]
     *
     * ainsi que MOV RIP-relative.
     */
    if module.len() < 7 {
        return results;
    }

    for offset in 0..module.len() - 7 {
        let rex = module[offset];

        if !(0x40..=0x4F).contains(&rex) {
            continue;
        }

        let opcode = module[offset + 1];

        if opcode != 0x8D && opcode != 0x8B {
            continue;
        }

        let modrm = module[offset + 2];

        /*
         * mod = 00
         * r/m = 101
         *
         * => RIP-relative.
         */
        if modrm & 0xC7 != 0x05 {
            continue;
        }

        let displacement = i32::from_le_bytes([
            module[offset + 3],
            module[offset + 4],
            module[offset + 5],
            module[offset + 6],
        ]);

        let instruction_end = module_base + offset + 7;

        let resolved = instruction_end.wrapping_add_signed(displacement as isize);

        if resolved == target {
            results.push(offset);
        }
    }

    results
}

fn dump_hex_context(module: &[u8], offset: usize) {
    let start = offset.saturating_sub(32);

    let end = (offset + 64).min(module.len());

    print!("    bytes client.dll+0x{start:X}: ");

    for byte in &module[start..end] {
        print!("{byte:02X} ");
    }

    println!();
}

fn dump_registration_targets(module: &[u8], module_base: usize, center: usize) {
    println!("[probe] Analysing RIP targets around client.dll+0x{center:X}");

    let start = center.saturating_sub(64);

    let end = (center + 96).min(module.len());

    let mut offset = start;

    while offset + 7 <= end {
        /*
         * LEA/MOV reg, [RIP + rel32]
         */
        let rex = module[offset];

        if (0x40..=0x4F).contains(&rex)
            && (module[offset + 1] == 0x8D || module[offset + 1] == 0x8B)
            && (module[offset + 2] & 0xC7) == 0x05
        {
            let displacement = i32::from_le_bytes([
                module[offset + 3],
                module[offset + 4],
                module[offset + 5],
                module[offset + 6],
            ]);

            let instruction_address = module_base + offset;

            let target = (instruction_address + 7).wrapping_add_signed(displacement as isize);

            println!(
                "[probe] RIP {:02X} {:02X} {:02X} at +0x{:X} -> 0x{:X}",
                module[offset],
                module[offset + 1],
                module[offset + 2],
                offset,
                target,
            );

            if target >= module_base && target < module_base + module.len() {
                let target_offset = target - module_base;

                println!("        target = client.dll+0x{target_offset:X}");

                let preview_end = (target_offset + 32).min(module.len());

                print!("        bytes  = ");

                for byte in &module[target_offset..preview_end] {
                    print!("{byte:02X} ");
                }

                println!();
            }
        }

        /*
         * CALL rel32
         */
        if module[offset] == 0xE8 && offset + 5 <= end {
            let displacement = i32::from_le_bytes([
                module[offset + 1],
                module[offset + 2],
                module[offset + 3],
                module[offset + 4],
            ]);

            let instruction_address = module_base + offset;

            let target = (instruction_address + 5).wrapping_add_signed(displacement as isize);

            println!("[probe] CALL at client.dll+0x{offset:X} -> 0x{target:X}");

            if target >= module_base && target < module_base + module.len() {
                let target_offset = target - module_base;

                println!("        call target = client.dll+0x{target_offset:X}");

                let preview_end = (target_offset + 32).min(module.len());

                print!("        bytes       = ");

                for byte in &module[target_offset..preview_end] {
                    print!("{byte:02X} ");
                }

                println!();
            }
        }

        offset += 1;
    }
}

fn scan_set_client_camera_angles(
    process: HANDLE,
    client_base: usize,
    client_size: usize,
) -> Result<(), String> {
    println!();
    println!("=== SetClientCameraAngles scan ===");

    println!("[probe] Reading client.dll for strings/xrefs...");

    let module = read_bytes(process, client_base, client_size)?;

    let search_strings = [
        "SetClientCameraAngles",
        "CCitadelUserMsg_SetClientCameraAngles",
        "CCitadelUserMsg_SetClientCameraAngles_t",
    ];

    let mut anything_found = false;

    for text in search_strings {
        let occurrences = find_all_bytes(&module, text.as_bytes());

        println!();
        println!("[probe] \"{text}\" -> {} occurrence(s)", occurrences.len(),);

        for string_offset in occurrences {
            anything_found = true;

            let string_address = client_base + string_offset;

            if text == "CCitadelUserMsg_SetClientCameraAngles_t" {
                if let Some((full_start, full_text)) = expand_ascii_string(&module, string_offset) {
                    let full_address = client_base + full_start;

                    println!("[probe] FULL ASCII: client.dll+0x{full_start:X}");

                    println!("[probe]   \"{full_text}\"");

                    /*
                     * La longue classe _Func_impl_no_alloc
                     * est polymorphique et doit donc avoir
                     * une vraie vtable MSVC.
                     *
                     * Elle contient la lambda de Dispatch
                     * que nous cherchons.
                     */
                    if full_text.contains("?$_Func_impl_no_alloc@")
                        && full_text.contains("CCitadelUserMsg_SetClientCameraAngles_t")
                    {
                        /*
                         * Le même full_text apparaît deux fois
                         * car le nom du message est présent
                         * plusieurs fois dans le symbole.
                         *
                         * On ne lance l'analyse que sur la
                         * première occurrence.
                         */
                        if let Some(first_match) =
                            full_text.find("CCitadelUserMsg_SetClientCameraAngles_t")
                        {
                            if string_offset == full_start + first_match {
                                inspect_msvc_rtti_vtable(
                                    &module,
                                    client_base,
                                    full_start,
                                    &full_text,
                                );
                            }
                        }
                    }

                    let full_xrefs = find_rip_xrefs(&module, client_base, full_address);

                    println!("[probe] FULL STRING RIP xrefs: {}", full_xrefs.len(),);

                    for full_xref in full_xrefs {
                        println!(
                            "[probe]   FULL XREF client.dll+0x{full_xref:X} (0x{:X})",
                            client_base + full_xref,
                        );

                        dump_hex_context(&module, full_xref);

                        dump_registration_targets(&module, client_base, full_xref);
                    }

                    let full_pointer_refs = find_absolute_pointer_refs(&module, full_address);

                    println!(
                        "[probe] FULL STRING pointer refs: {}",
                        full_pointer_refs.len(),
                    );

                    for pointer_ref in full_pointer_refs {
                        println!(
                            "[probe]   FULL PTRREF client.dll+0x{pointer_ref:X} (0x{:X})",
                            client_base + pointer_ref,
                        );

                        dump_pointer_neighborhood(&module, client_base, pointer_ref);
                    }
                }
            }

            println!("[probe] STRING: client.dll+0x{string_offset:X} (0x{string_address:X})");

            let xrefs = find_rip_xrefs(&module, client_base, string_address);

            println!("[probe] RIP xrefs: {}", xrefs.len(),);

            for xref in xrefs {
                println!(
                    "[probe]   XREF client.dll+0x{xref:X} (0x{:X})",
                    client_base + xref,
                );

                dump_hex_context(&module, xref);

                dump_registration_targets(&module, client_base, xref);
            }

            let pointer_refs = find_absolute_pointer_refs(&module, string_address);

            println!("[probe] Absolute pointer refs: {}", pointer_refs.len(),);

            for pointer_ref in pointer_refs.iter().take(20) {
                println!(
                    "[probe]   PTRREF client.dll+0x{:X} (0x{:X})",
                    pointer_ref,
                    client_base + pointer_ref,
                );

                dump_pointer_neighborhood(&module, client_base, *pointer_ref);
            }
        }
    }

    if !anything_found {
        println!("[probe] Neither camera message string was found.");
    }

    println!();
    println!("=== scan finished ===");

    Ok(())
}

fn find_pattern(data: &[u8], pattern: &[Option<u8>]) -> Option<usize> {
    if pattern.is_empty() || pattern.len() > data.len() {
        return None;
    }

    'outer: for start in 0..=data.len() - pattern.len() {
        for (offset, expected) in pattern.iter().enumerate() {
            if let Some(expected) = expected {
                if data[start + offset] != *expected {
                    continue 'outer;
                }
            }
        }

        return Some(start);
    }

    None
}

fn resolve_prediction(
    process: HANDLE,
    client_base: usize,
    client_size: usize,
) -> Result<usize, String> {
    println!(
        "[probe] Reading client.dll ({:.1} MB)...",
        client_size as f64 / 1024.0 / 1024.0,
    );

    let module = read_bytes(process, client_base, client_size)?;

    let pattern = parse_pattern(PREDICTION_PATTERN)?;

    let offset = find_pattern(&module, &pattern).ok_or_else(|| {
        "Prediction pattern was not found. Valve probably changed this code path.".to_string()
    })?;

    /*
     * 48 8D 05 XX XX XX XX
     *
     * LEA RAX, [RIP + rel32]
     *
     * instruction length = 7
     */
    let relative = i32::from_le_bytes([
        module[offset + 3],
        module[offset + 4],
        module[offset + 5],
        module[offset + 6],
    ]);

    let instruction = client_base + offset;

    let resolved = (instruction + 7).wrapping_add_signed(relative as isize);

    println!("[probe] Prediction signature: client.dll+0x{:X}", offset,);

    println!("[probe] Prediction object: 0x{resolved:X}",);

    Ok(resolved)
}

fn qangle_from_bytes(data: &[u8], offset: usize) -> Option<QAngle> {
    if offset + 12 > data.len() {
        return None;
    }

    Some(QAngle {
        pitch: f32::from_le_bytes(data[offset..offset + 4].try_into().ok()?),

        yaw: f32::from_le_bytes(data[offset + 4..offset + 8].try_into().ok()?),

        roll: f32::from_le_bytes(data[offset + 8..offset + 12].try_into().ok()?),
    })
}

fn angle_delta(from: f32, to: f32) -> f32 {
    let mut delta = to - from;

    while delta > 180.0 {
        delta -= 360.0;
    }

    while delta < -180.0 {
        delta += 360.0;
    }

    delta
}

fn capture_pawn_memory(process: HANDLE, pawn: usize) -> Result<Vec<u8>, String> {
    /*
     * On couvre largement la zone des
     * champs C_CitadelPlayerPawn.
     */
    read_bytes(process, pawn, 0x2200)
}

fn wait_for_enter(message: &str) {
    println!();
    println!("{message}");
    println!("Puis reviens ici et appuie sur Entrée.");

    let mut input = String::new();

    let _ = std::io::stdin().read_line(&mut input);
}

fn scan_camera_candidates(first: &[u8], horizontal: &[u8], vertical: &[u8]) {
    #[derive(Debug)]
    struct Candidate {
        offset: usize,

        a: QAngle,
        b: QAngle,
        c: QAngle,

        score: f32,
    }

    let mut candidates = Vec::<Candidate>::new();

    /*
     * On commence à 0x800 pour éviter
     * toute la partie base entity/vtable.
     */
    for offset in (0x800..0x2100).step_by(4) {
        let Some(a) = qangle_from_bytes(first, offset) else {
            continue;
        };

        let Some(b) = qangle_from_bytes(horizontal, offset) else {
            continue;
        };

        let Some(c) = qangle_from_bytes(vertical, offset) else {
            continue;
        };

        if !plausible_angle(a) || !plausible_angle(b) || !plausible_angle(c) {
            continue;
        }

        /*
         * Capture A -> B :
         * tu tourneras surtout horizontalement.
         */
        let horizontal_yaw = angle_delta(a.yaw, b.yaw).abs();

        let horizontal_pitch = angle_delta(a.pitch, b.pitch).abs();

        /*
         * Capture B -> C :
         * tu bougeras surtout verticalement.
         */
        let vertical_pitch = angle_delta(b.pitch, c.pitch).abs();

        let vertical_yaw = angle_delta(b.yaw, c.yaw).abs();

        let roll_change = (a.roll - b.roll).abs() + (b.roll - c.roll).abs();

        /*
         * Les seuils restent volontairement
         * assez tolérants : tu ne peux pas
         * déplacer une souris parfaitement
         * sur un seul axe.
         */
        if horizontal_yaw < 8.0 {
            continue;
        }

        if vertical_pitch < 4.0 {
            continue;
        }

        if horizontal_pitch > 30.0 {
            continue;
        }

        if vertical_yaw > 40.0 {
            continue;
        }

        if roll_change > 15.0 {
            continue;
        }

        let score = horizontal_yaw * 2.0 + vertical_pitch * 2.0
            - horizontal_pitch
            - vertical_yaw
            - roll_change;

        candidates.push(Candidate {
            offset,
            a,
            b,
            c,
            score,
        });
    }

    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    println!();
    println!("================ CAMERA CANDIDATES ================");

    if candidates.is_empty() {
        println!("Aucun candidat trouvé.");

        return;
    }

    for candidate in candidates.iter().take(20) {
        println!();
        println!(
            "OFFSET 0x{:X}   score {:.2}",
            candidate.offset, candidate.score,
        );

        println!(
            "  A : pitch={:>8.3} yaw={:>9.3} roll={:>8.3}",
            candidate.a.pitch, candidate.a.yaw, candidate.a.roll,
        );

        println!(
            "  B : pitch={:>8.3} yaw={:>9.3} roll={:>8.3}",
            candidate.b.pitch, candidate.b.yaw, candidate.b.roll,
        );

        println!(
            "  C : pitch={:>8.3} yaw={:>9.3} roll={:>8.3}",
            candidate.c.pitch, candidate.c.yaw, candidate.c.roll,
        );
    }
}

fn plausible_angle(angle: QAngle) -> bool {
    angle.pitch.is_finite()
        && angle.yaw.is_finite()
        && angle.roll.is_finite()
        && angle.pitch.abs() < 1000.0
        && angle.yaw.abs() < 10000.0
        && angle.roll.abs() < 1000.0
}

fn find_pawn_scene_node(process: HANDLE, pawn: usize) -> Result<(usize, usize), String> {
    /*
     * On ne dépend pas de l'offset
     * m_pGameSceneNode dans C_BaseEntity.
     *
     * On cherche un pointeur contenu
     * dans le pawn dont :
     *
     * candidate + 0x30 = pawn
     *
     * car CGameSceneNode::m_pOwner
     * est à +0x30.
     */
    for pawn_offset in (0..0x500).step_by(8) {
        let Ok(candidate) = read_value::<usize>(process, pawn + pawn_offset) else {
            continue;
        };

        if candidate < 0x10000 {
            continue;
        }

        let Ok(owner) = read_value::<usize>(process, candidate + SCENE_NODE_OWNER_OFFSET) else {
            continue;
        };

        if owner == pawn {
            return Ok((candidate, pawn_offset));
        }
    }

    Err("Could not locate pawn CGameSceneNode".to_string())
}

fn probe_scene_node_facing(process: HANDLE, pawn: usize) -> Result<(), String> {
    println!();
    println!("=== SCENE NODE FACING PROBE ===");

    let (scene_node, pawn_offset) = find_pawn_scene_node(process, pawn)?;

    println!("[probe] Scene node = 0x{scene_node:X}");

    println!("[probe] Found through pawn+0x{pawn_offset:X}");

    let initial_local: QAngle = read_value(process, scene_node + SCENE_NODE_LOCAL_ROTATION)?;

    let initial_abs: QAngle = read_value(process, scene_node + SCENE_NODE_ABS_ROTATION)?;

    println!(
        "[probe] INITIAL local -> P={:.3} Y={:.3} R={:.3}",
        initial_local.pitch, initial_local.yaw, initial_local.roll,
    );

    println!(
        "[probe] INITIAL abs   -> P={:.3} Y={:.3} R={:.3}",
        initial_abs.pitch, initial_abs.yaw, initial_abs.roll,
    );

    println!();
    println!("F6 = mémoriser les rotations");
    println!("F7 = écrire m_angRotation");
    println!("F8 = écrire m_angAbsRotation");
    println!("F9 = écrire LES DEUX");
    println!("Ctrl+C = quitter");
    println!();

    let mut saved_local: Option<QAngle> = None;

    let mut saved_abs: Option<QAngle> = None;

    let mut old_f6 = false;
    let mut old_f7 = false;
    let mut old_f8 = false;
    let mut old_f9 = false;

    loop {
        let down = |vk: i32| (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0;

        let f6 = down(VK_F6 as i32);

        let f7 = down(VK_F7 as i32);

        let f8 = down(VK_F8 as i32);

        let f9 = down(VK_F9 as i32);

        if f6 && !old_f6 {
            let local: QAngle = read_value(process, scene_node + SCENE_NODE_LOCAL_ROTATION)?;

            let absolute: QAngle = read_value(process, scene_node + SCENE_NODE_ABS_ROTATION)?;

            saved_local = Some(local);

            saved_abs = Some(absolute);

            println!("[probe] SAVED");

            println!(
                "  local -> P={:.3} Y={:.3} R={:.3}",
                local.pitch, local.yaw, local.roll,
            );

            println!(
                "  abs   -> P={:.3} Y={:.3} R={:.3}",
                absolute.pitch, absolute.yaw, absolute.roll,
            );
        }

        if f7 && !old_f7 {
            if let Some(value) = saved_local {
                write_value(process, scene_node + SCENE_NODE_LOCAL_ROTATION, &value)?;

                println!("[probe] WROTE m_angRotation");
            }
        }

        if f8 && !old_f8 {
            if let Some(value) = saved_abs {
                write_value(process, scene_node + SCENE_NODE_ABS_ROTATION, &value)?;

                println!("[probe] WROTE m_angAbsRotation");
            }
        }

        if f9 && !old_f9 {
            if let Some(value) = saved_local {
                write_value(process, scene_node + SCENE_NODE_LOCAL_ROTATION, &value)?;
            }

            if let Some(value) = saved_abs {
                write_value(process, scene_node + SCENE_NODE_ABS_ROTATION, &value)?;
            }

            println!("[probe] WROTE BOTH scene rotations");
        }

        old_f6 = f6;
        old_f7 = f7;
        old_f8 = f8;
        old_f9 = f9;

        thread::sleep(Duration::from_millis(2));
    }
}

fn probe_pawn_facing(process: HANDLE, local_pawn: usize) -> Result<(), String> {
    println!();
    println!("=== PAWN FACING PROBE ===");

    println!("[probe] Local pawn = 0x{local_pawn:X}");

    println!();
    println!("F6 = mémoriser les angles actuels");
    println!("F7 = écrire m_angEyeAngles");
    println!("F8 = écrire v_angle + v_anglePrevious");
    println!("F9 = écrire m_angClientCamera");
    println!("Ctrl+C = quitter");
    println!();

    let mut saved_eye: Option<QAngle> = None;
    let mut saved_view: Option<QAngle> = None;
    let mut saved_client: Option<QAngle> = None;

    let mut old_f6 = false;
    let mut old_f7 = false;
    let mut old_f8 = false;
    let mut old_f9 = false;

    loop {
        let down = |vk: i32| (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0;

        let f6 = down(VK_F6 as i32);
        let f7 = down(VK_F7 as i32);
        let f8 = down(VK_F8 as i32);
        let f9 = down(VK_F9 as i32);

        if f6 && !old_f6 {
            let eye: QAngle = read_value(process, local_pawn + ANG_EYE_ANGLES)?;

            let view: QAngle = read_value(process, local_pawn + V_ANGLE)?;

            let client: QAngle = read_value(process, local_pawn + ANG_CLIENT_CAMERA)?;

            saved_eye = Some(eye);
            saved_view = Some(view);
            saved_client = Some(client);

            println!("[probe] SAVED");
            println!(
                "  eye    -> P={:.3} Y={:.3} R={:.3}",
                eye.pitch, eye.yaw, eye.roll,
            );
            println!(
                "  view   -> P={:.3} Y={:.3} R={:.3}",
                view.pitch, view.yaw, view.roll,
            );
            println!(
                "  client -> P={:.3} Y={:.3} R={:.3}",
                client.pitch, client.yaw, client.roll,
            );
        }

        if f7 && !old_f7 {
            if let Some(value) = saved_eye {
                write_value(process, local_pawn + ANG_EYE_ANGLES, &value)?;

                println!("[probe] WROTE m_angEyeAngles");
            }
        }

        if f8 && !old_f8 {
            if let Some(value) = saved_view {
                write_value(process, local_pawn + V_ANGLE, &value)?;

                write_value(process, local_pawn + V_ANGLE_PREVIOUS, &value)?;

                println!("[probe] WROTE v_angle + v_anglePrevious");
            }
        }

        if f9 && !old_f9 {
            if let Some(value) = saved_client {
                write_value(process, local_pawn + ANG_CLIENT_CAMERA, &value)?;

                println!("[probe] WROTE m_angClientCamera");
            }
        }

        old_f6 = f6;
        old_f7 = f7;
        old_f8 = f8;
        old_f9 = f9;

        thread::sleep(Duration::from_millis(2));
    }
}

fn probe_full_camera_transform(process: HANDLE, client_base: usize) -> Result<(), String> {
    println!();
    println!("=== FULL CAMERA TRANSFORM PROBE ===");

    let global = client_base + THIRD_PERSON_CAMERA_GLOBAL;

    let camera_object: usize = read_value(process, global)?;

    if camera_object == 0 {
        return Err("Camera object is NULL".to_string());
    }

    println!("[probe] Camera object = 0x{camera_object:X}");

    let mut saved_position: Option<Vec3> = None;

    let mut saved_angles: Option<QAngle> = None;

    let mut old_f6 = false;
    let mut old_f7 = false;

    println!();
    println!("F6 = sauvegarder position + angles caméra");
    println!("F7 = restaurer position + angles caméra");
    println!("Ctrl+C = quitter");
    println!();

    loop {
        let f6 = (unsafe { GetAsyncKeyState(VK_F6 as i32) } as u16 & 0x8000) != 0;

        let f7 = (unsafe { GetAsyncKeyState(VK_F7 as i32) } as u16 & 0x8000) != 0;

        if f6 && !old_f6 {
            let position: Vec3 = read_value(process, camera_object + CAMERA_POSITION_OFFSET)?;

            let angles: QAngle = read_value(process, camera_object + CAMERA_ANGLES_OFFSET)?;

            saved_position = Some(position);

            saved_angles = Some(angles);

            println!("[probe] SAVED");

            println!(
                "  position -> X={:.3} Y={:.3} Z={:.3}",
                position.x, position.y, position.z,
            );

            println!(
                "  angles   -> P={:.3} Y={:.3} R={:.3}",
                angles.pitch, angles.yaw, angles.roll,
            );
        }

        if f7 && !old_f7 {
            if let (Some(position), Some(angles)) = (saved_position, saved_angles) {
                write_value(process, camera_object + CAMERA_POSITION_OFFSET, &position)?;

                write_value(process, camera_object + CAMERA_ANGLES_OFFSET, &angles)?;

                println!("[probe] WROTE full camera transform");

                let after_position: Vec3 =
                    read_value(process, camera_object + CAMERA_POSITION_OFFSET)?;

                let after_angles: QAngle =
                    read_value(process, camera_object + CAMERA_ANGLES_OFFSET)?;

                println!(
                    "  after position -> X={:.3} Y={:.3} Z={:.3}",
                    after_position.x, after_position.y, after_position.z,
                );

                println!(
                    "  after angles   -> P={:.3} Y={:.3} R={:.3}",
                    after_angles.pitch, after_angles.yaw, after_angles.roll,
                );
            }
        }

        old_f6 = f6;
        old_f7 = f7;

        thread::sleep(Duration::from_millis(2));
    }
}

fn probe_old_view_angles(process: HANDLE, pawn: usize) -> Result<(), String> {
    println!();
    println!("=== OLD VIEW ANGLES PROBE ===");

    let movement_services: usize = read_value(process, pawn + MOVEMENT_SERVICES_OFFSET)?;

    if movement_services == 0 {
        return Err("Movement services pointer is NULL".to_string());
    }

    println!("[probe] Movement services = 0x{movement_services:X}");

    let mut saved: Option<QAngle> = None;

    let mut old_f6 = false;
    let mut old_f7 = false;
    let mut old_f8 = false;

    println!();
    println!("F6 = mémoriser m_vecOldViewAngles");
    println!("F7 = écriture unique");
    println!("F8 = maintenir la valeur pendant 40 ms");
    println!();

    loop {
        let down = |vk: i32| (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0;

        let f6 = down(VK_F6 as i32);
        let f7 = down(VK_F7 as i32);
        let f8 = down(VK_F8 as i32);

        if f6 && !old_f6 {
            let value: QAngle = read_value(process, movement_services + OLD_VIEW_ANGLES_OFFSET)?;

            saved = Some(value);

            println!(
                "[probe] SAVED old view -> P={:.3} Y={:.3} R={:.3}",
                value.pitch, value.yaw, value.roll,
            );
        }

        if f7 && !old_f7 {
            if let Some(value) = saved {
                write_value(process, movement_services + OLD_VIEW_ANGLES_OFFSET, &value)?;

                println!("[probe] WROTE old view once");
            }
        }

        if f8 && !old_f8 {
            if let Some(value) = saved {
                let start = std::time::Instant::now();

                while start.elapsed() < Duration::from_millis(40) {
                    write_value(process, movement_services + OLD_VIEW_ANGLES_OFFSET, &value)?;

                    thread::sleep(Duration::from_millis(1));
                }

                println!("[probe] HELD old view for 40 ms");
            }
        }

        old_f6 = f6;
        old_f7 = f7;
        old_f8 = f8;

        thread::sleep(Duration::from_millis(2));
    }
}

fn probe_anim_graph(process: HANDLE, pawn: usize) -> Result<(), String> {
    println!();
    println!("=== ANIM GRAPH PROBE ===");

    let initial: u8 = read_value(process, pawn + ANIM_GRAPH_UPDATE_ENABLED)?;

    println!("[probe] m_bAnimGraphUpdateEnabled = {}", initial,);

    println!();
    println!("Maintiens F6 = désactiver AnimGraph");
    println!("Relâche F6 = réactiver AnimGraph");
    println!("Ctrl+C = quitter");
    println!();

    let mut disabled = false;

    loop {
        let f6 = (unsafe { GetAsyncKeyState(VK_F6 as i32) } as u16 & 0x8000) != 0;

        if f6 && !disabled {
            let zero: u8 = 0;

            write_value(process, pawn + ANIM_GRAPH_UPDATE_ENABLED, &zero)?;

            disabled = true;

            println!("[probe] AnimGraph DISABLED");
        }

        if !f6 && disabled {
            let one: u8 = 1;

            write_value(process, pawn + ANIM_GRAPH_UPDATE_ENABLED, &one)?;

            disabled = false;

            println!("[probe] AnimGraph ENABLED");
        }

        thread::sleep(Duration::from_millis(2));
    }
}

fn run() -> Result<(), String> {
    println!("=== SPLIT camera_probe ===");

    println!("[probe] Looking for deadlock.exe...");

    let pid = find_process_id(PROCESS_NAME)?;

    println!("[probe] Deadlock PID: {pid}");

    let (client_base, client_size) = find_module(pid, "client.dll")?;

    println!("[probe] client.dll: 0x{client_base:X} / 0x{client_size:X} bytes");

    let process = open_deadlock(pid)?;

    let result = (|| -> Result<(), String> {
        let prediction = resolve_prediction(process, client_base, client_size)?;

        let pawn: usize = read_value(process, prediction + LOCAL_PAWN_FROM_PREDICTION)?;

        if pawn == 0 {
            return Err(
                "Local pawn pointer is NULL. Enter Hero Sandbox / practice first.".to_string(),
            );
        }

        println!("[probe] Local pawn: 0x{pawn:X}");

        probe_anim_graph(process, pawn)?;

        Ok(())
    })();

    unsafe {
        CloseHandle(process);
    }

    result
}

fn main() {
    if let Err(error) = run() {
        eprintln!();
        eprintln!("[probe] ERROR: {error}");

        eprintln!();
        eprintln!("Appuie sur Entrée pour fermer.");

        let mut input = String::new();

        let _ = std::io::stdin().read_line(&mut input);
    }
}
