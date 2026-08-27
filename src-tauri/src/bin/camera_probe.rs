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
        Threading::{
            OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ,
            PROCESS_VM_WRITE,
        },
        UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F6, VK_F7},
    },
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

/*
 * Utile pour comparaison.
 *
 * Les recherches précédentes donnent
 * v_angle autour de 0xF98.
 */
const V_ANGLE: usize = 0xF98;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct QAngle {
    pitch: f32,
    yaw: f32,
    roll: f32,
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

        println!();
        println!("=== NATIVE CAMERA WRITE TEST ===");

        println!("F6 = mémoriser la caméra actuelle");

        println!("F7 = réécrire instantanément la caméra mémorisée");

        println!("Ctrl+C = quitter");

        println!();

        let camera_address = pawn + ANG_CLIENT_CAMERA;

        println!("[probe] m_angClientCamera address: 0x{camera_address:X}");

        let mut saved_camera: Option<QAngle> = None;

        let mut f6_was_down = false;

        let mut f7_was_down = false;

        loop {
            let current: QAngle = read_value(process, camera_address)?;

            let f6_down = (unsafe { GetAsyncKeyState(VK_F6 as i32) } as u16 & 0x8000) != 0;

            let f7_down = (unsafe { GetAsyncKeyState(VK_F7 as i32) } as u16 & 0x8000) != 0;

            /*
             * F6 : mémoriser exactement
             * m_angClientCamera.
             */
            if f6_down && !f6_was_down {
                saved_camera = Some(current);

                println!(
                    "[probe] SAVED -> pitch={:.3} yaw={:.3} roll={:.3}",
                    current.pitch, current.yaw, current.roll,
                );
            }

            /*
             * F7 : écrire le QAngle mémorisé
             * directement dans Deadlock.
             */
            if f7_down && !f7_was_down {
                if let Some(target) = saved_camera {
                    println!(
                        "[probe] WRITE -> pitch={:.3} yaw={:.3} roll={:.3}",
                        target.pitch, target.yaw, target.roll,
                    );

                    write_value(process, camera_address, &target)?;

                    /*
                     * Vérifie immédiatement ce
                     * qu'il y a en mémoire.
                     */
                    let immediate: QAngle = read_value(process, camera_address)?;

                    println!(
                        "[probe] immediately after write -> P={:.3} Y={:.3} R={:.3}",
                        immediate.pitch, immediate.yaw, immediate.roll,
                    );

                    thread::sleep(Duration::from_millis(20));

                    let after_20ms: QAngle = read_value(process, camera_address)?;

                    println!(
                        "[probe] 20 ms later          -> P={:.3} Y={:.3} R={:.3}",
                        after_20ms.pitch, after_20ms.yaw, after_20ms.roll,
                    );

                    thread::sleep(Duration::from_millis(100));

                    let after_120ms: QAngle = read_value(process, camera_address)?;

                    println!(
                        "[probe] 120 ms later         -> P={:.3} Y={:.3} R={:.3}",
                        after_120ms.pitch, after_120ms.yaw, after_120ms.roll,
                    );
                } else {
                    println!("[probe] F7 ignored: press F6 first");
                }
            }

            f6_was_down = f6_down;

            f7_was_down = f7_down;

            thread::sleep(Duration::from_millis(5));
        }
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
