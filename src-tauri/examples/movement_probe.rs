use std::{
    ffi::c_void,
    mem::{size_of, MaybeUninit},
    thread,
    time::{Duration, Instant},
};

use sysinfo::{ProcessesToUpdate, System};

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
            GetProcessId, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION,
            PROCESS_VM_READ, PROCESS_VM_WRITE,
        },
    },
    UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F11, VK_F12},
};

/*
 * C_BaseEntity
 */
const BODY_COMPONENT: usize = 0x30;

const SCENE_NODE: usize = 0x330;
const COLLISION_PROPERTY: usize = 0x340;

const ENTITY_STATE_BASE: usize = 0x400;

const FLAGS: usize = 0x400;

const ABS_VELOCITY: usize = 0x404;
const SERVER_VELOCITY: usize = 0x410;
const VELOCITY: usize = 0x438;

// C_CitadelPlayerPawn
const EYE_ANGLES: usize = 0x11B8;
const CLIENT_CAMERA_ANGLES: usize = 0x1248;
const LOCKED_EYE_ANGLES: usize = 0x14C0;

/*
 * CBaseAnimGraph
 */
const GRAPH_CONTROLLER_MANAGER: usize = 0x9A8;
const MAIN_GRAPH_CONTROLLER: usize = 0xA58;

const ABILITY_REQUIRES_DEBOUNCE: usize = 0x1188;

const ABILITY_COMPONENT: usize = 0x1468;

const ABILITY_VECTOR: usize = ABILITY_COMPONENT + 0x68;

const SELECTED_ABILITY: usize = ABILITY_COMPONENT + 0xC8;

const CHANNELLING_ABILITY: usize = ABILITY_COMPONENT + 0xCC;

const CAST_DELAYING_ABILITY: usize = ABILITY_COMPONENT + 0xD0;

const PREVIOUS_ABILITY_QUEUED: usize = ABILITY_COMPONENT + 0xD8;

const ABILITY_INTERRUPT_STATE: usize = ABILITY_COMPONENT + 0xE4;

const EXECUTE_ABILITY_MASK: usize = ABILITY_COMPONENT + 0x1D8;

const LAST_VELOCITY: usize = 0x175C;

const QUEUED_ABILITY: usize = 0x1880;
const QUEUED_ABILITY_END_TIME: usize = 0x1888;

const ANIM_MOVEMENT_CLIPPED: usize = 0x1890;
const ANIM_MOVEMENT_DISABLE_GRAVITY: usize = 0x1891;
const ANIM_MOVEMENT_DIRECT_AIR_CONTROL: usize = 0x1892;

const MOVE_TYPE: usize = 0x521;
const ACTUAL_MOVE_TYPE: usize = 0x522;

const GROUND_ENTITY: usize = 0x52C;
const GROUND_BODY_INDEX: usize = 0x530;

const GRAVITY_SCALE: usize = 0x53C;
const GRAVITY_DISABLED: usize = 0x545;

const ACTUAL_GRAVITY_SCALE: usize = 0x55C;
const GRAVITY_ACTUALLY_DISABLED: usize = 0x560;

/*
 * C_BasePlayerPawn
 */
const MOVEMENT_SERVICES: usize = 0xF28;

/*
 * CBodyComponentBaseAnimGraph
 */
const BODY_ANIM_CONTROLLER: usize = 0x4D0;

/*
 * CGameSceneNode
 */
const ABS_ORIGIN: usize = 0xC8;

const SCENE_LOCAL_ANGLES: usize = 0xB8;
const SCENE_ABS_ANGLES: usize = 0xD4;

/*
 * CPlayer_MovementServices_Humanoid
 */
const FALL_VELOCITY: usize = 0x244;
const GROUND_NORMAL: usize = 0x248;

/*
 * CCitadelPlayer_MovementServices
 */
const POSITION_DELTA_VELOCITY: usize = 0x270;

/*
 * CNetworkVelocityVector
 * Chaque CNetworkedQuantizedFloat contient sa float en premier.
 */
const NETWORK_VELOCITY_X: usize = 0x10;
const NETWORK_VELOCITY_Y: usize = 0x18;
const NETWORK_VELOCITY_Z: usize = 0x20;

const TOGGLE_DUCK_ACTIVE: usize = 0x2A0;
const DUCKED: usize = 0x2A1;

const POGO_VELOCITY: usize = 0x2A4;
const SUPPORT: usize = 0x2B0;
const COLLIDING: usize = 0x2BC;
const LANDED_ON_GROUND: usize = 0x2BD;

/*
 * CCollisionProperty
 */
const COLLISION_MINS: usize = 0x40;
const COLLISION_MAXS: usize = 0x4C;

const SOLID_FLAGS: usize = 0x5A;
const SOLID_TYPE: usize = 0x5B;
const PHYSICS_ENABLED: usize = 0x5F;

const CAPSULE_CENTER_1: usize = 0x94;
const CAPSULE_CENTER_2: usize = 0xA0;
const CAPSULE_RADIUS: usize = 0xAC;

const COLLISION_STATE_SIZE: usize = 0xB0;

/*
 * Prediction -> local pawn
 */
const LOCAL_PAWN_IN_PREDICTION: usize = 0xD8;

/*
 * On lit ENTITY_STATE_BASE..=0x563
 * en une seule fois pour limiter énormément
 * les appels ReadProcessMemory.
 */
const ENTITY_STATE_SIZE: usize = 0x564 - ENTITY_STATE_BASE;

/*
 * MovementServices :
 * de m_flFallVelocity à m_bLandedOnGround.
 */
const MOVEMENT_STATE_SIZE: usize = (LANDED_ON_GROUND + 1) - FALL_VELOCITY;

const PAWN_YAW_SCAN_SIZE: usize = 0x2000;

const CAPTURE_DURATION: Duration = Duration::from_millis(6000);

const SAMPLE_DELAY: Duration = Duration::from_millis(1);

#[derive(Debug, Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

struct YawScanBaseline {
    pawn: usize,
    yaw: f32,
    bytes: Vec<u8>,
}

struct GraphInstanceBaseline {
    instance: usize,
    scene_yaw: f32,
    camera_yaw: f32,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    ms: u128,

    pawn: usize,

    position: Vec3,

    scene_yaw: f32,
    camera_yaw: f32,

    flags: u32,
    on_ground: bool,

    velocity: Vec3,
    server_velocity: Vec3,
    network_velocity: Vec3,

    move_type: u8,
    actual_move_type: u8,

    ground_entity: u32,
    ground_body_index: i32,

    gravity_scale: f32,
    gravity_disabled: bool,

    actual_gravity_scale: f32,
    gravity_actually_disabled: bool,

    fall_velocity: f32,

    position_delta_velocity: Vec3,
    pogo_velocity: Vec3,

    ground_normal: Vec3,

    last_velocity: Vec3,

    ability_requires_debounce: u32,
    selected_ability: u32,
    channelling_ability: u32,
    cast_delaying_ability: u32,

    previous_ability_queued: u8,
    ability_interrupt_state: u8,
    execute_ability_mask: u32,

    queued_ability: u64,
    queued_ability_end_time: f32,

    anim_movement_clipped: u8,
    anim_movement_disable_gravity: u8,
    anim_movement_direct_air_control: u8,

    support: Vec3,

    toggle_duck_active: bool,
    ducked: bool,

    colliding: bool,
    landed_on_ground: bool,

    collision_mins: Vec3,
    collision_maxs: Vec3,

    solid_flags: u8,
    solid_type: u8,
    physics_enabled: u8,

    capsule_center_1: Vec3,
    capsule_center_2: Vec3,
    capsule_radius: f32,
}
struct ProcessHandle(HANDLE);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn deadlock_pid() -> Option<u32> {
    let mut system = System::new();

    system.refresh_processes(ProcessesToUpdate::All, true);

    system.processes().iter().find_map(|(pid, process)| {
        process
            .name()
            .to_string_lossy()
            .eq_ignore_ascii_case("deadlock.exe")
            .then(|| pid.as_u32())
    })
}

fn open_deadlock(pid: u32) -> Result<ProcessHandle, String> {
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION,
            0,
            pid,
        )
    };

    if handle.is_null() {
        return Err("Impossible d'ouvrir deadlock.exe".to_string());
    }

    Ok(ProcessHandle(handle))
}

fn wide_to_string(value: &[u16]) -> String {
    let length = value
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(value.len());

    String::from_utf16_lossy(&value[..length])
}

fn client_module(pid: u32) -> Result<(usize, usize), String> {
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };

    if snapshot == INVALID_HANDLE_VALUE {
        return Err("CreateToolhelp32Snapshot a échoué".to_string());
    }

    let mut entry: MODULEENTRY32W = unsafe { std::mem::zeroed() };

    entry.dwSize = size_of::<MODULEENTRY32W>() as u32;

    let mut found = None;

    let mut success = unsafe { Module32FirstW(snapshot, &mut entry) };

    while success != 0 {
        let name = wide_to_string(&entry.szModule);

        if name.eq_ignore_ascii_case("client.dll") {
            found = Some((entry.modBaseAddr as usize, entry.modBaseSize as usize));

            break;
        }

        success = unsafe { Module32NextW(snapshot, &mut entry) };
    }

    unsafe {
        CloseHandle(snapshot);
    }

    found.ok_or_else(|| "client.dll introuvable".to_string())
}

fn module_containing_address(
    process: HANDLE,
    address: usize,
) -> Result<Option<(String, usize, usize)>, String> {
    let pid = unsafe { GetProcessId(process) };

    if pid == 0 {
        return Err("GetProcessId a échoué".to_string());
    }

    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };

    if snapshot == INVALID_HANDLE_VALUE {
        return Err("CreateToolhelp32Snapshot a échoué".to_string());
    }

    let mut entry: MODULEENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<MODULEENTRY32W>() as u32;

    let mut result = None;

    let mut success = unsafe { Module32FirstW(snapshot, &mut entry) };

    while success != 0 {
        let base = entry.modBaseAddr as usize;
        let size = entry.modBaseSize as usize;
        let end = base.saturating_add(size);

        if address >= base && address < end {
            result = Some((wide_to_string(&entry.szModule), base, size));

            break;
        }

        success = unsafe { Module32NextW(snapshot, &mut entry) };
    }

    unsafe {
        CloseHandle(snapshot);
    }

    Ok(result)
}

fn read_value<T: Copy>(process: HANDLE, address: usize) -> Result<T, String> {
    let mut value = MaybeUninit::<T>::uninit();

    let mut read = 0usize;

    let success = unsafe {
        ReadProcessMemory(
            process,
            address as *const c_void,
            value.as_mut_ptr() as *mut c_void,
            size_of::<T>(),
            &mut read,
        )
    };

    if success == 0 || read != size_of::<T>() {
        return Err(format!("ReadProcessMemory failed @ 0x{address:X}"));
    }

    Ok(unsafe { value.assume_init() })
}

fn write_value<T: Copy>(process: HANDLE, address: usize, value: &T) -> Result<(), String> {
    let mut written = 0usize;

    let success = unsafe {
        WriteProcessMemory(
            process,
            address as *mut c_void,
            value as *const T as *const c_void,
            size_of::<T>(),
            &mut written,
        )
    };

    if success == 0 || written != size_of::<T>() {
        return Err(format!("WriteProcessMemory failed @ 0x{address:X}"));
    }

    Ok(())
}

fn read_bytes(process: HANDLE, address: usize, size: usize) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0u8; size];

    let mut read = 0usize;

    let success = unsafe {
        ReadProcessMemory(
            process,
            address as *const c_void,
            buffer.as_mut_ptr() as *mut c_void,
            size,
            &mut read,
        )
    };

    if success == 0 || read != size {
        return Err(format!("ReadProcessMemory failed @ 0x{address:X}"));
    }

    Ok(buffer)
}

fn clear_last_velocity_once(_process: HANDLE, _prediction: usize) -> Result<(), String> {
    Ok(())
}

fn force_can_dash_jump_once(
    process: HANDLE,
    prediction: usize,
    entity_list: usize,
) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    /*
     * Layout runtime validé :
     *
     * pawn + 0x14D0 = CCitadelAbilityComponent
     * component + 0x68 = m_vecAbilities
     */
    let component = pawn + 0x14D0;
    let abilities_vector = component + 0x68;

    let count = read_value::<u64>(process, abilities_vector + 0x00)?;

    let data = read_value::<u64>(process, abilities_vector + 0x08)?;

    if count == 0 || count > 64 || data < 0x10000 {
        return Err("m_vecAbilities suspect".to_string());
    }

    let buffer = read_bytes(process, data as usize, count as usize * size_of::<u32>())?;

    let mut jump_instance = None;

    for i in 0..count as usize {
        let offset = i * size_of::<u32>();

        let handle = u32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap());

        let Some((_resolved_index, _chunk, identity, instance, stored_handle)) =
            try_resolve_entity_with_stride(process, entity_list, handle, 0x70)
        else {
            continue;
        };

        if stored_handle != handle {
            continue;
        }

        let designer_ptr = read_value::<usize>(process, identity + 0x20).unwrap_or(0);

        let designer = read_c_string_lossy(process, designer_ptr, 96);

        if designer == "citadel_ability_jump" {
            jump_instance = Some(instance);
            break;
        }
    }

    let jump = jump_instance.ok_or_else(|| "citadel_ability_jump introuvable".to_string())?;

    /*
     * Offset runtime validé dans nos captures :
     * CCitadel_Ability_Jump::m_LastJumpType
     */
    /*
     * Test causal :
     *
     * CCitadel_Ability_Jump::m_bCanDashJump
     * runtime actuel = +0x1581
     */
    let address = jump + 0x1581;

    let before = read_value::<u8>(process, address)?;

    println!();
    println!("[F11] jump instance = 0x{jump:X}");
    println!("[F11] canDashJump avant = {before}");

    /*
     * Dans notre comparaison :
     *
     * A sain = 1
     * B bug  = 0
     *
     * On refuse donc d'écrire si on n'est pas
     * exactement dans l'état B attendu.
     */
    if before != 0 {
        return Err(format!("Test annulé : canDashJump vaut {before}, pas 0"));
    }

    let forced = 1u8;

    write_value(process, address, &forced)?;

    let after = read_value::<u8>(process, address)?;

    println!("[F11] canDashJump après = {after}");

    if after != forced {
        return Err(format!("Écriture non confirmée : attendu 1, lu {after}"));
    }

    println!("[F11] TEST ACTIF : canDashJump 0 -> 1");

    Ok(())
}

fn u32_at(buffer: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap())
}

fn i32_at(buffer: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap())
}

fn f32_at(buffer: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap())
}

fn vec3_at(buffer: &[u8], offset: usize) -> Vec3 {
    Vec3 {
        x: f32_at(buffer, offset),
        y: f32_at(buffer, offset + 4),
        z: f32_at(buffer, offset + 8),
    }
}

fn network_velocity_at(buffer: &[u8], offset: usize) -> Vec3 {
    Vec3 {
        x: f32_at(buffer, offset + NETWORK_VELOCITY_X),
        y: f32_at(buffer, offset + NETWORK_VELOCITY_Y),
        z: f32_at(buffer, offset + NETWORK_VELOCITY_Z),
    }
}

fn prediction_signature_matches(data: &[u8], offset: usize) -> bool {
    if offset + 21 > data.len() {
        return false;
    }

    data[offset] == 0x48
        && data[offset + 1] == 0x8D
        && data[offset + 2] == 0x05
        /*
         * +3..+7 = disp32
         */
        && data[offset + 7] == 0xC3
        && data[offset + 8..offset + 16]
            .iter()
            .all(|byte| *byte == 0xCC)
        && data[offset + 16] == 0x40
        && data[offset + 17] == 0x53
        && data[offset + 18] == 0x56
        && data[offset + 19] == 0x41
        && data[offset + 20] == 0x54
}

fn resolve_prediction(
    process: HANDLE,
    client_base: usize,
    client_size: usize,
) -> Result<usize, String> {
    println!(
        "[probe] Lecture de client.dll uniquement ({:.1} MB)...",
        client_size as f64 / 1024.0 / 1024.0,
    );

    let client = read_bytes(process, client_base, client_size)?;

    let mut candidates = Vec::new();

    for offset in 0..client.len().saturating_sub(21) {
        if !prediction_signature_matches(&client, offset) {
            continue;
        }

        let displacement =
            i32::from_le_bytes(client[offset + 3..offset + 7].try_into().unwrap()) as isize;

        let rip = client_base.wrapping_add(offset).wrapping_add(7);

        let target = rip.wrapping_add_signed(displacement);

        candidates.push(target);
    }

    if candidates.is_empty() {
        return Err("Signature Prediction introuvable".to_string());
    }

    println!("[probe] Prediction candidates = {}", candidates.len());

    /*
     * On privilégie un candidat qui possède
     * déjà un local pawn valide.
     */
    for prediction in &candidates {
        let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION).unwrap_or(0);

        if pawn > 0x10000 {
            println!("[probe] Prediction = 0x{prediction:X}");

            println!("[probe] Local pawn = 0x{pawn:X}");

            return Ok(*prediction);
        }
    }

    /*
     * Si on est dans un état où le pawn
     * est temporairement nul, conserver le
     * candidat unique reste utile.
     */
    if candidates.len() == 1 {
        println!("[probe] Prediction = 0x{:X}", candidates[0],);

        return Ok(candidates[0]);
    }

    Err("Plusieurs Prediction candidates et aucun local pawn valide".to_string())
}

fn entity_list_signature_matches(data: &[u8], offset: usize) -> bool {
    if offset + 16 > data.len() {
        return false;
    }

    /*
     * 48 8B 0D ?? ?? ?? ??
     * 48 89 7C 24 ??
     * 8B FA
     * C1 EB
     */
    data[offset] == 0x48
        && data[offset + 1] == 0x8B
        && data[offset + 2] == 0x0D
        && data[offset + 7] == 0x48
        && data[offset + 8] == 0x89
        && data[offset + 9] == 0x7C
        && data[offset + 10] == 0x24
        && data[offset + 12] == 0x8B
        && data[offset + 13] == 0xFA
        && data[offset + 14] == 0xC1
        && data[offset + 15] == 0xEB
}

fn resolve_entity_list(
    process: HANDLE,
    client_base: usize,
    client_size: usize,
) -> Result<usize, String> {
    /*
     * Scan limité à client.dll.
     * Pas de full-process scan.
     */
    let client = read_bytes(process, client_base, client_size)?;

    let mut candidates = Vec::new();

    for offset in 0..=client.len().saturating_sub(16) {
        if !entity_list_signature_matches(&client, offset) {
            continue;
        }

        let displacement =
            i32::from_le_bytes(client[offset + 3..offset + 7].try_into().unwrap()) as isize;

        /*
         * 48 8B 0D disp32
         *
         * RIP est à la fin de l'instruction, donc +7.
         */
        let rip = client_base.wrapping_add(offset).wrapping_add(7);

        let global = rip.wrapping_add_signed(displacement);

        let entity_list = read_value::<usize>(process, global).unwrap_or(0);

        if entity_list >= 0x10000 {
            candidates.push((global, entity_list));
        }
    }

    if candidates.len() != 1 {
        return Err(format!(
            "Entity List signature: {} candidat(s) valide(s)",
            candidates.len()
        ));
    }

    let (global, entity_list) = candidates[0];

    println!();
    println!("[ability] entity_list_global=0x{global:X}");
    println!("[ability] entity_list=0x{entity_list:X}");

    Ok(entity_list)
}

fn read_c_string_lossy(process: HANDLE, address: usize, max_length: usize) -> String {
    if address < 0x10000 {
        return "<null>".to_string();
    }

    let Ok(buffer) = read_bytes(process, address, max_length) else {
        return "<unreadable>".to_string();
    };

    let length = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());

    String::from_utf8_lossy(&buffer[..length]).into_owned()
}

fn try_resolve_entity_with_stride(
    process: HANDLE,
    entity_list: usize,
    handle: u32,
    stride: usize,
) -> Option<(usize, usize, usize, usize, u32)> {
    /*
     * Deadlock : 14-bit entity index.
     */
    let index = (handle & 0x3FFF) as usize;

    /*
     * 512 identities par chunk.
     */
    let chunk_index = index >> 9;
    let slot_index = index & 0x1FF;

    /*
     * CConcreteEntityList :
     *
     * +0x10 = premier pointeur de chunk
     * puis 8 octets par chunk.
     */
    let chunk = read_value::<usize>(
        process,
        entity_list + 0x10 + chunk_index * size_of::<usize>(),
    )
    .ok()?;

    if chunk < 0x10000 {
        return None;
    }

    let identity = chunk + slot_index * stride;

    /*
     * Le premier qword du slot doit pointer
     * vers la CEntityInstance.
     */
    let instance = read_value::<usize>(process, identity).ok()?;

    if instance < 0x10000 {
        return None;
    }

    /*
     * Validation extrêmement importante :
     *
     * CEntityInstance::m_pEntity est à +0x10.
     *
     * Il doit repointer EXACTEMENT vers le
     * CEntityIdentity que nous venons de calculer.
     */
    let back_identity = read_value::<usize>(process, instance + 0x10).ok()?;

    if back_identity != identity {
        return None;
    }

    /*
     * CEntityIdentity contient également son handle
     * autour de +0x10 dans le layout runtime Source 2.
     *
     * On le log mais on ne l'utilise PAS encore
     * comme condition de validation.
     */
    let stored_handle = read_value::<u32>(process, identity + 0x10).unwrap_or(0);

    Some((index, chunk, identity, instance, stored_handle))
}

fn dump_jump_state(process: HANDLE, jump: usize) -> Result<(), String> {
    println!();
    println!("[jump] ========================================");
    println!("[jump] instance=0x{jump:X}");

    /*
     * Layout candidat beaucoup plus récent :
     * deadlock-dumper, dump du 7 mai 2026.
     *
     * Toujours READ-ONLY.
     */

    let last_zip = read_value::<f32>(process, jump + 0x11D8)?;

    let last_ground = read_value::<f32>(process, jump + 0x11DC)?;

    let phase_start = read_value::<f32>(process, jump + 0x11E0)?;

    let jump_time = read_value::<f32>(process, jump + 0x11E4)?;

    let wall_fatigue_start = read_value::<f32>(process, jump + 0x11E8)?;

    let last_think = read_value::<f32>(process, jump + 0x11EC)?;

    println!("[jump] lastZip          @11D8 = {last_zip:.6}");
    println!("[jump] lastGround       @11DC = {last_ground:.6}");
    println!("[jump] phaseStart       @11E0 = {phase_start:.6}");
    println!("[jump] jumpTime         @11E4 = {jump_time:.6}");
    println!("[jump] wallFatigueStart @11E8 = {wall_fatigue_start:.6}");
    println!("[jump] lastThink        @11EC = {last_think:.6}");

    /*
     * Wall/jump state.
     */
    let last_wall_jump = read_value::<f32>(process, jump + 0x1220)?;

    let wall_facing = read_value::<i16>(process, jump + 0x1230)?;

    let wall_fatigue_strength = read_value::<f32>(process, jump + 0x1234)?;

    let last_jump_type = read_value::<u8>(process, jump + 0x1238)?;

    let create_air_effects = read_value::<u8>(process, jump + 0x1239)?;

    let double_jump_fail_time = read_value::<f32>(process, jump + 0x123C)?;

    let double_jump_fail_reason = read_value::<i32>(process, jump + 0x1240)?;

    println!();
    println!("[jump] lastWallJump       @1220 = {last_wall_jump:.6}");
    println!("[jump] wallFacing         @1230 = {wall_facing}");
    println!("[jump] wallFatigueStrength@1234 = {wall_fatigue_strength:.6}");
    println!("[jump] lastJumpType       @1238 = {last_jump_type}");
    println!("[jump] createAirEffects   @1239 = {create_air_effects}");
    println!("[jump] doubleJumpFailTime @123C = {double_jump_fail_time:.6}");
    println!("[jump] doubleJumpFailWhy  @1240 = {double_jump_fail_reason}");

    /*
     * Layout client Deadlock récent :
     * Source revision 10905930, build 12 août 2026.
     */

    let dash_start_q00 = read_value::<u64>(process, jump + 0x1550)?;

    let dash_start_time = read_value::<f32>(process, jump + 0x1558)?;

    let dash_end_q00 = read_value::<u64>(process, jump + 0x1568)?;

    let dash_end_time = read_value::<f32>(process, jump + 0x1570)?;

    println!();
    println!("[jump] dashStart raw      @1550 = 0x{dash_start_q00:016X}");
    println!("[jump] dashStart time     @1558 = {dash_start_time:.6}");
    println!("[jump] dashEnd raw        @1568 = 0x{dash_end_q00:016X}");
    println!("[jump] dashEnd time       @1570 = {dash_end_time:.6}");

    let jumped = read_value::<u8>(process, jump + 0x1580)?;

    let can_dash_jump = read_value::<u8>(process, jump + 0x1581)?;

    let desired_air_jumps = read_value::<i32>(process, jump + 0x1584)?;

    let executed_air_jumps = read_value::<i32>(process, jump + 0x1588)?;

    let in_slide_jump = read_value::<u8>(process, jump + 0x158C)?;

    let consecutive_air_jumps = read_value::<i8>(process, jump + 0x158D)?;

    let consecutive_wall_jumps = read_value::<i8>(process, jump + 0x158E)?;

    let lateral_suppress_end = read_value::<f32>(process, jump + 0x1590)?;

    println!();
    println!("[jump] jumped             @1580 = {jumped}");
    println!("[jump] canDashJump        @1581 = {can_dash_jump}");
    println!("[jump] desiredAirJumps    @1584 = {desired_air_jumps}");
    println!("[jump] executedAirJumps   @1588 = {executed_air_jumps}");
    println!("[jump] inSlideJump        @158C = {in_slide_jump}");
    println!("[jump] consecutiveAir     @158D = {consecutive_air_jumps}");
    println!("[jump] consecutiveWall    @158E = {consecutive_wall_jumps}");
    println!("[jump] lateralSuppressEnd @1590 = {lateral_suppress_end:.6}");
    println!();
    println!("[jump] === raw 11D8..11F0 ===");

    for offset in (0x11D8usize..0x11F0usize).step_by(4) {
        let raw = read_value::<u32>(process, jump + offset)?;

        println!(
            "[jump] +0x{offset:04X} \
             raw=0x{raw:08X} \
             f32={:.6}",
            f32::from_bits(raw)
        );
    }

    println!();
    println!("[jump] === raw 1550..1598 ===");

    for offset in (0x1550usize..0x1598usize).step_by(4) {
        let raw = read_value::<u32>(process, jump + offset)?;

        println!(
            "[jump] +0x{offset:04X} \
            raw=0x{raw:08X} \
            f32={:.6}",
            f32::from_bits(raw)
        );
    }

    for offset in (0x14D0usize..0x1518usize).step_by(4) {
        let raw = read_value::<u32>(process, jump + offset)?;

        println!(
            "[jump] +0x{offset:04X} \
             raw=0x{raw:08X} \
             f32={:.6}",
            f32::from_bits(raw)
        );
    }

    println!("[jump] ========================================");
    println!();

    Ok(())
}

fn dump_ability_handles(
    process: HANDLE,
    prediction: usize,
    entity_list: usize,
) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    /*
     * Validé par les probes précédents :
     *
     * pawn + 0x14D0 = CCitadelAbilityComponent runtime
     * component + 0x68 = m_vecAbilities
     * component + 0x80 = m_vecThinkableAbilities
     */
    let component = pawn + 0x14D0;

    let abilities_vector = component + 0x68;
    let thinkable_vector = component + 0x80;

    println!();
    println!("[ability] pawn=0x{pawn:X}");
    println!("[ability] component=0x{component:X}");

    let ability_count = read_value::<u64>(process, abilities_vector + 0x00)?;
    let ability_data = read_value::<u64>(process, abilities_vector + 0x08)?;
    let ability_capacity = read_value::<u64>(process, abilities_vector + 0x10)?;

    println!();
    println!("[ability] === m_vecAbilities ===");
    println!("[ability] vector=0x{abilities_vector:X}");
    println!("[ability] count={ability_count}");
    println!("[ability] data=0x{ability_data:016X}");
    println!("[ability] capacity={ability_capacity}");

    let thinkable_count = read_value::<u64>(process, thinkable_vector + 0x00)?;
    let thinkable_data = read_value::<u64>(process, thinkable_vector + 0x08)?;
    let thinkable_capacity = read_value::<u64>(process, thinkable_vector + 0x10)?;

    println!();
    println!("[ability] === m_vecThinkableAbilities ===");
    println!("[ability] count={thinkable_count}");
    println!("[ability] data=0x{thinkable_data:016X}");
    println!("[ability] capacity={thinkable_capacity}");

    if ability_count == 0 || ability_count > 64 || ability_data < 0x10000 {
        return Err("m_vecAbilities suspect".to_string());
    }

    let count = ability_count as usize;

    let buffer = read_bytes(process, ability_data as usize, count * size_of::<u32>())?;

    println!();
    println!("[ability] === resolved ability entities ===");

    for index_in_vector in 0..count {
        let offset = index_in_vector * size_of::<u32>();

        let handle = u32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap());

        let entity_index = handle & 0x3FFF;
        let serial = handle >> 14;

        /*
         * Le stride 0x70 est maintenant validé au runtime.
         */
        let Some((resolved_index, _chunk, identity, instance, stored_handle)) =
            try_resolve_entity_with_stride(process, entity_list, handle, 0x70)
        else {
            println!(
                "[ability] [{index_in_vector:02}] \
             handle=0x{handle:08X} \
             index=0x{entity_index:04X} \
             RESOLVE FAILED"
            );

            continue;
        };

        /*
         * Validation stricte :
         * l'identity doit contenir exactement
         * le CHandle qu'on est en train de résoudre.
         */
        if stored_handle != handle {
            println!(
                "[ability] [{index_in_vector:02}] \
             handle=0x{handle:08X} \
             WRONG IDENTITY HANDLE=0x{stored_handle:08X}"
            );

            continue;
        }

        let designer_name_ptr = read_value::<usize>(process, identity + 0x20).unwrap_or(0);

        let designer_name = read_c_string_lossy(process, designer_name_ptr, 96);

        let jump_marker = if designer_name.to_ascii_lowercase() == "citadel_ability_jump" {
            "  <=== JUMP?"
        } else {
            ""
        };

        println!(
            "[ability] [{index_in_vector:02}] \
         handle=0x{handle:08X} \
         idx=0x{resolved_index:04X} \
         serial={serial} \
         instance=0x{instance:X} \
         designer=\"{designer_name}\"\
         {jump_marker}"
        );

        if designer_name == "citadel_ability_jump" {
            if let Err(error) = dump_jump_state(process, instance) {
                eprintln!("[jump] ERROR: {error}");
            }
        }
    }

    println!();

    Ok(())
}

fn read_sample(process: HANDLE, prediction: usize, started: Instant) -> Result<Sample, String> {
    /*
     * Important :
     * on relit le pawn à chaque sample.
     */
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    let scene_node = read_value::<usize>(process, pawn + SCENE_NODE)?;

    let movement = read_value::<usize>(process, pawn + MOVEMENT_SERVICES)?;

    let collision = read_value::<usize>(process, pawn + COLLISION_PROPERTY)?;

    if scene_node < 0x10000 {
        return Err("Scene node nul".to_string());
    }

    if movement < 0x10000 {
        return Err("MovementServices nul".to_string());
    }

    if collision < 0x10000 {
        return Err("CollisionProperty nulle".to_string());
    }

    let position = read_value::<Vec3>(process, scene_node + ABS_ORIGIN)?;

    let scene_angles = read_value::<Vec3>(process, scene_node + SCENE_LOCAL_ANGLES)?;

    let camera_angles = read_value::<Vec3>(process, pawn + CLIENT_CAMERA_ANGLES)?;

    /*
     * Un seul read pour la majorité
     * des champs C_BaseEntity.
     */
    let entity = read_bytes(process, pawn + ENTITY_STATE_BASE, ENTITY_STATE_SIZE)?;

    /*
     * Un seul read pour le bloc mouvement
     * qui nous intéresse.
     */
    let movement_state = read_bytes(process, movement + FALL_VELOCITY, MOVEMENT_STATE_SIZE)?;

    let collision_state = read_bytes(process, collision, COLLISION_STATE_SIZE)?;

    let flags = u32_at(&entity, FLAGS - ENTITY_STATE_BASE);

    let velocity = vec3_at(&entity, ABS_VELOCITY - ENTITY_STATE_BASE);

    let server_velocity = network_velocity_at(&entity, SERVER_VELOCITY - ENTITY_STATE_BASE);

    let network_velocity = network_velocity_at(&entity, VELOCITY - ENTITY_STATE_BASE);

    let move_type = entity[MOVE_TYPE - ENTITY_STATE_BASE];

    let actual_move_type = entity[ACTUAL_MOVE_TYPE - ENTITY_STATE_BASE];

    let ground_entity = u32_at(&entity, GROUND_ENTITY - ENTITY_STATE_BASE);

    let ground_body_index = i32_at(&entity, GROUND_BODY_INDEX - ENTITY_STATE_BASE);

    let gravity_scale = f32_at(&entity, GRAVITY_SCALE - ENTITY_STATE_BASE);

    let gravity_disabled = entity[GRAVITY_DISABLED - ENTITY_STATE_BASE] != 0;

    let actual_gravity_scale = f32_at(&entity, ACTUAL_GRAVITY_SCALE - ENTITY_STATE_BASE);

    let gravity_actually_disabled = entity[GRAVITY_ACTUALLY_DISABLED - ENTITY_STATE_BASE] != 0;

    let fall_velocity = f32_at(&movement_state, 0);

    let position_delta_velocity =
        network_velocity_at(&movement_state, POSITION_DELTA_VELOCITY - FALL_VELOCITY);

    let pogo_velocity = vec3_at(&movement_state, POGO_VELOCITY - FALL_VELOCITY);

    let ground_normal = vec3_at(&movement_state, GROUND_NORMAL - FALL_VELOCITY);

    let support = vec3_at(&movement_state, SUPPORT - FALL_VELOCITY);

    let last_velocity = read_value::<Vec3>(process, pawn + LAST_VELOCITY)?;

    let ability_requires_debounce = read_value::<u32>(process, pawn + ABILITY_REQUIRES_DEBOUNCE)?;

    let selected_ability = read_value::<u32>(process, pawn + SELECTED_ABILITY)?;

    let channelling_ability = read_value::<u32>(process, pawn + CHANNELLING_ABILITY)?;

    let cast_delaying_ability = read_value::<u32>(process, pawn + CAST_DELAYING_ABILITY)?;

    let previous_ability_queued = read_value::<u8>(process, pawn + PREVIOUS_ABILITY_QUEUED)?;

    let ability_interrupt_state = read_value::<u8>(process, pawn + ABILITY_INTERRUPT_STATE)?;

    let execute_ability_mask = read_value::<u32>(process, pawn + EXECUTE_ABILITY_MASK)?;

    let queued_ability = read_value::<u64>(process, pawn + QUEUED_ABILITY)?;

    let queued_ability_end_time = read_value::<f32>(process, pawn + QUEUED_ABILITY_END_TIME)?;

    let anim_movement_clipped = read_value::<u8>(process, pawn + ANIM_MOVEMENT_CLIPPED)?;

    let anim_movement_disable_gravity =
        read_value::<u8>(process, pawn + ANIM_MOVEMENT_DISABLE_GRAVITY)?;

    let anim_movement_direct_air_control =
        read_value::<u8>(process, pawn + ANIM_MOVEMENT_DIRECT_AIR_CONTROL)?;

    let colliding = movement_state[COLLIDING - FALL_VELOCITY] != 0;

    let landed_on_ground = movement_state[LANDED_ON_GROUND - FALL_VELOCITY] != 0;

    let toggle_duck_active = movement_state[TOGGLE_DUCK_ACTIVE - FALL_VELOCITY] != 0;

    let ducked = movement_state[DUCKED - FALL_VELOCITY] != 0;

    let collision_mins = vec3_at(&collision_state, COLLISION_MINS);

    let collision_maxs = vec3_at(&collision_state, COLLISION_MAXS);

    let solid_flags = collision_state[SOLID_FLAGS];

    let solid_type = collision_state[SOLID_TYPE];

    let physics_enabled = collision_state[PHYSICS_ENABLED];

    let capsule_center_1 = vec3_at(&collision_state, CAPSULE_CENTER_1);

    let capsule_center_2 = vec3_at(&collision_state, CAPSULE_CENTER_2);

    let capsule_radius = f32_at(&collision_state, CAPSULE_RADIUS);

    Ok(Sample {
        ms: started.elapsed().as_millis(),

        pawn,

        position,

        scene_yaw: scene_angles.y,
        camera_yaw: camera_angles.y,

        flags,
        on_ground: flags & 0x1 != 0,

        velocity,
        server_velocity,
        network_velocity,

        move_type,
        actual_move_type,

        ground_entity,
        ground_body_index,

        gravity_scale,
        gravity_disabled,

        actual_gravity_scale,
        gravity_actually_disabled,

        fall_velocity,

        position_delta_velocity,
        pogo_velocity,

        ground_normal,

        last_velocity,

        ability_requires_debounce,
        selected_ability,
        channelling_ability,
        cast_delaying_ability,

        previous_ability_queued,
        ability_interrupt_state,
        execute_ability_mask,

        queued_ability,
        queued_ability_end_time,

        anim_movement_clipped,
        anim_movement_disable_gravity,
        anim_movement_direct_air_control,

        support,

        toggle_duck_active,
        ducked,

        colliding,
        landed_on_ground,

        collision_mins,
        collision_maxs,

        solid_flags,
        solid_type,
        physics_enabled,

        capsule_center_1,
        capsule_center_2,
        capsule_radius,
    })
}

fn move_type_name(value: u8) -> &'static str {
    match value {
        0 => "NONE",
        1 => "OBSOLETE",
        2 => "WALK",
        3 => "FLY",
        4 => "FLYGRAV",
        5 => "VPHYS",
        6 => "PUSH",
        7 => "NOCLIP",
        8 => "OBSERVER",
        9 => "STEP",
        10 => "SYNC",
        11 => "CUSTOM",
        _ => "?",
    }
}

fn discrete_changed(previous: &Sample, current: &Sample) -> bool {
    previous.pawn != current.pawn
        || previous.flags != current.flags
        || previous.move_type != current.move_type
        || previous.actual_move_type != current.actual_move_type
        || previous.ground_entity != current.ground_entity
        || previous.ground_body_index != current.ground_body_index
        || previous.gravity_disabled != current.gravity_disabled
        || previous.gravity_actually_disabled != current.gravity_actually_disabled
        || previous.colliding != current.colliding
        || previous.landed_on_ground != current.landed_on_ground
}

fn print_sample(sample: &Sample) {
    println!(
        "{:>4}ms | XYZ={:>8.3},{:>8.3},{:>8.3} | \
absV={:>7.2},{:>7.2},{:>7.2} \
srvV={:>7.2},{:>7.2},{:>7.2} \
netV={:>7.2},{:>7.2},{:>7.2} | \
fall={:>7.2} delta={:>7.2},{:>7.2},{:>7.2} \
pogo={:>7.2},{:>7.2},{:>7.2} | \
support={:>7.2},{:>7.2},{:>7.2} \
gN={:>6.2},{:>6.2},{:>6.2} |
ground={} hGround=0x{:08X} | \
move={}/{} | duck={}/{} | \
hullZ={:.2}..{:.2} | \
capsuleZ={:.2}..{:.2} r={:.2} | \
solid={}/{} physics={} | coll={} landed={}",
        sample.ms,
        sample.position.x,
        sample.position.y,
        sample.position.z,
        sample.velocity.x,
        sample.velocity.y,
        sample.velocity.z,
        sample.server_velocity.x,
        sample.server_velocity.y,
        sample.server_velocity.z,
        sample.network_velocity.x,
        sample.network_velocity.y,
        sample.network_velocity.z,
        sample.fall_velocity,
        sample.position_delta_velocity.x,
        sample.position_delta_velocity.y,
        sample.position_delta_velocity.z,
        sample.pogo_velocity.x,
        sample.pogo_velocity.y,
        sample.pogo_velocity.z,
        sample.support.x,
        sample.support.y,
        sample.support.z,
        sample.ground_normal.x,
        sample.ground_normal.y,
        sample.ground_normal.z,
        if sample.on_ground { "YES" } else { " NO" },
        sample.ground_entity,
        sample.move_type,
        sample.actual_move_type,
        sample.toggle_duck_active as u8,
        sample.ducked as u8,
        sample.collision_mins.z,
        sample.collision_maxs.z,
        sample.capsule_center_1.z,
        sample.capsule_center_2.z,
        sample.capsule_radius,
        sample.solid_type,
        sample.solid_flags,
        sample.physics_enabled,
        sample.colliding as u8,
        sample.landed_on_ground as u8,
    );

    println!(
        "     hidden | \
    lastV={:>7.2},{:>7.2},{:>7.2} | \
    debounce=0x{:08X} sel=0x{:08X} chan=0x{:08X} cast=0x{:08X} | \
    prevQ={} intr={} exec=0x{:08X} | \
    queued=0x{:016X} qEnd={:.3} | \
    anim={}/{}/{}",
        sample.last_velocity.x,
        sample.last_velocity.y,
        sample.last_velocity.z,
        sample.ability_requires_debounce,
        sample.selected_ability,
        sample.channelling_ability,
        sample.cast_delaying_ability,
        sample.previous_ability_queued,
        sample.ability_interrupt_state,
        sample.execute_ability_mask,
        sample.queued_ability,
        sample.queued_ability_end_time,
        sample.anim_movement_clipped,
        sample.anim_movement_disable_gravity,
        sample.anim_movement_direct_air_control,
    );
}

fn dump_scene_angles_once(process: HANDLE, prediction: usize) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    let scene_node = read_value::<usize>(process, pawn + SCENE_NODE)?;

    if scene_node < 0x10000 {
        return Err("Scene node nul".to_string());
    }

    let local = read_value::<Vec3>(process, scene_node + SCENE_LOCAL_ANGLES)?;

    let absolute = read_value::<Vec3>(process, scene_node + SCENE_ABS_ANGLES)?;

    println!();
    println!("[ANGLE] pawn      = 0x{pawn:X}");
    println!("[ANGLE] sceneNode = 0x{scene_node:X}");

    println!(
        "[ANGLE] local +0xB8 = {:.3}, {:.3}, {:.3}",
        local.x, local.y, local.z
    );

    println!(
        "[ANGLE] abs   +0xD4 = {:.3}, {:.3}, {:.3}",
        absolute.x, absolute.y, absolute.z
    );

    println!();

    Ok(())
}

fn force_scene_local_yaw_once(process: HANDLE, prediction: usize) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    let scene_node = read_value::<usize>(process, pawn + SCENE_NODE)?;

    if scene_node < 0x10000 {
        return Err("Scene node nul".to_string());
    }

    let address = scene_node + SCENE_LOCAL_ANGLES;

    let before = read_value::<Vec3>(process, address)?;

    println!();
    println!(
        "[ANGLE TEST] local avant = {:.3}, {:.3}, {:.3}",
        before.x, before.y, before.z
    );

    let forced = Vec3 {
        x: before.x,
        y: -179.8125,
        z: before.z,
    };

    write_value(process, address, &forced)?;

    let after = read_value::<Vec3>(process, address)?;

    println!(
        "[ANGLE TEST] local après = {:.3}, {:.3}, {:.3}",
        after.x, after.y, after.z
    );

    Ok(())
}

fn scan_pawn_yaw_candidates(
    process: HANDLE,
    prediction: usize,
    baseline: &mut Option<YawScanBaseline>,
) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    let scene_node = read_value::<usize>(process, pawn + SCENE_NODE)?;

    if scene_node < 0x10000 {
        return Err("Scene node nul".to_string());
    }

    /*
     * On utilise uniquement SceneNode +0xB8
     * comme référence READ-ONLY du yaw actuel.
     *
     * Nos tests précédents ont montré qu'il
     * correspond exactement à getpos_exact.
     */
    let scene_angles = read_value::<Vec3>(process, scene_node + SCENE_LOCAL_ANGLES)?;

    let yaw = scene_angles.y;

    let bytes = read_bytes(process, pawn, PAWN_YAW_SCAN_SIZE)?;

    if baseline.is_none() {
        println!();
        println!("========== YAW SCAN BASELINE ==========");
        println!("[YAW] pawn = 0x{pawn:X}");
        println!("[YAW] référence = {yaw:.6}");
        println!("[YAW] Tourne maintenant d'environ 90° puis F11 une deuxième fois.");
        println!();

        *baseline = Some(YawScanBaseline { pawn, yaw, bytes });

        return Ok(());
    }

    let first = baseline
        .take()
        .ok_or_else(|| "Baseline absente".to_string())?;

    if first.pawn != pawn {
        return Err("Le local pawn a changé entre les deux captures".to_string());
    }

    println!();
    println!("========== YAW SCAN RESULTS ==========");
    println!(
        "[YAW] référence A = {:.6} | référence B = {:.6}",
        first.yaw, yaw
    );

    let mut matches = 0usize;

    for offset in (0..PAWN_YAW_SCAN_SIZE - 4).step_by(4) {
        let before = f32::from_le_bytes(first.bytes[offset..offset + 4].try_into().unwrap());

        let after = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());

        if !before.is_finite() || !after.is_finite() {
            continue;
        }

        let matches_before = (before - first.yaw).abs() <= 0.25;
        let matches_after = (after - yaw).abs() <= 0.25;

        if matches_before && matches_after {
            println!(
                "[YAW CANDIDATE] pawn+0x{offset:04X} : {:.6} -> {:.6}",
                before, after
            );

            matches += 1;
        }
    }

    println!("[YAW] {matches} candidat(s)");
    println!("======================================");
    println!();

    Ok(())
}

fn dump_pawn_angles_once(process: HANDLE, prediction: usize) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    let scene_node = read_value::<usize>(process, pawn + SCENE_NODE)?;

    if scene_node < 0x10000 {
        return Err("Scene node nul".to_string());
    }

    let scene = read_value::<Vec3>(process, scene_node + SCENE_LOCAL_ANGLES)?;

    let eye = read_value::<Vec3>(process, pawn + EYE_ANGLES)?;

    let client_camera = read_value::<Vec3>(process, pawn + CLIENT_CAMERA_ANGLES)?;

    let locked_eye = read_value::<Vec3>(process, pawn + LOCKED_EYE_ANGLES)?;

    println!();
    println!("========== PAWN ANGLES ==========");
    println!(
        "[ANGLES] SceneNode = {:.3}, {:.3}, {:.3}",
        scene.x, scene.y, scene.z
    );
    println!(
        "[ANGLES] Eye       = {:.3}, {:.3}, {:.3}",
        eye.x, eye.y, eye.z
    );
    println!(
        "[ANGLES] ClientCam = {:.3}, {:.3}, {:.3}",
        client_camera.x, client_camera.y, client_camera.z
    );
    println!(
        "[ANGLES] LockedEye = {:.3}, {:.3}, {:.3}",
        locked_eye.x, locked_eye.y, locked_eye.z
    );
    println!("=================================");
    println!();

    Ok(())
}

fn force_client_camera_yaw_once(process: HANDLE, prediction: usize) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    let address = pawn + CLIENT_CAMERA_ANGLES;

    let before = read_value::<Vec3>(process, address)?;

    println!();
    println!(
        "[CLIENTCAM TEST] avant = {:.3}, {:.3}, {:.3}",
        before.x, before.y, before.z
    );

    /*
     * m_angClientCamera utilise ici des angles 0..360.
     *
     * Notre yaw de slot -179.8125° correspond à :
     * 180.1875°
     */
    let forced = Vec3 {
        x: before.x,
        y: 180.1875,
        z: before.z,
    };

    write_value(process, address, &forced)?;

    let after = read_value::<Vec3>(process, address)?;

    println!(
        "[CLIENTCAM TEST] après = {:.3}, {:.3}, {:.3}",
        after.x, after.y, after.z
    );

    Ok(())
}

fn rtti_name_from_object(
    process: HANDLE,
    client_base: usize,
    client_size: usize,
    object: usize,
) -> String {
    if object < 0x10000 {
        return "<null>".to_string();
    }

    let Ok(vtable) = read_value::<usize>(process, object) else {
        return "<vtable unreadable>".to_string();
    };

    let client_end = client_base.saturating_add(client_size);

    if vtable < client_base || vtable >= client_end {
        return format!("<vtable 0x{vtable:X} hors client.dll>");
    }

    if vtable < 8 {
        return "<invalid vtable>".to_string();
    }

    /*
     * MSVC x64 :
     *
     * vtable[-1] = RTTICompleteObjectLocator*
     */
    let Ok(col) = read_value::<usize>(process, vtable - 8) else {
        return "<COL unreadable>".to_string();
    };

    if col < client_base || col >= client_end {
        return format!("<COL 0x{col:X} hors client.dll>");
    }

    /*
     * RTTICompleteObjectLocator +0x0C
     * = RVA du TypeDescriptor.
     */
    let Ok(type_rva) = read_value::<u32>(process, col + 0x0C) else {
        return "<TypeDescriptor RVA unreadable>".to_string();
    };

    let type_descriptor = client_base + type_rva as usize;

    if type_descriptor < client_base || type_descriptor >= client_end {
        return "<TypeDescriptor hors client.dll>".to_string();
    }

    /*
     * TypeDescriptor +0x10 = nom MSVC :
     * ".?AVClassName@@"
     */
    read_c_string_lossy(process, type_descriptor + 0x10, 160)
}

fn dump_graph_controllers_once(
    process: HANDLE,
    prediction: usize,
    client_base: usize,
    client_size: usize,
) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    let manager = pawn + GRAPH_CONTROLLER_MANAGER;

    let main = read_value::<usize>(process, pawn + MAIN_GRAPH_CONTROLLER)?;

    /*
     * CAnimGraphControllerManager::m_controllers
     * est un CUtlVector occupant 0x18 octets.
     *
     * On log d'abord le layout brut avant
     * de faire confiance au count.
     */
    let count = read_value::<u64>(process, manager + 0x00)?;
    let data = read_value::<usize>(process, manager + 0x08)?;
    let capacity = read_value::<u64>(process, manager + 0x10)?;

    println!();
    println!("========== ANIM GRAPH CONTROLLERS ==========");
    println!("[GRAPH] pawn    = 0x{pawn:X}");
    println!("[GRAPH] manager = 0x{manager:X}");
    println!("[GRAPH] vector: count={count} data=0x{data:X} capacity={capacity}");

    println!(
        "[GRAPH] main = 0x{main:X} | {}",
        rtti_name_from_object(process, client_base, client_size, main,)
    );

    /*
     * Refus de parcourir si le layout ne ressemble
     * pas à un CUtlVector valide.
     */
    if data < 0x10000 {
        return Err("m_controllers data invalide".to_string());
    }

    if count == 0 || count > 64 {
        return Err(format!("m_controllers count suspect : {count}"));
    }

    println!();
    println!("[GRAPH] controllers :");

    for index in 0..count as usize {
        let controller = read_value::<usize>(process, data + index * size_of::<usize>())?;

        let name = rtti_name_from_object(process, client_base, client_size, controller);

        let external_graph = if controller >= 0x10000 {
            read_value::<u32>(process, controller + 0x18).unwrap_or(0xFFFF_FFFF)
        } else {
            0xFFFF_FFFF
        };

        println!(
            "[GRAPH] [{index:02}] 0x{controller:X} \
             ext=0x{external_graph:08X} | {name}"
        );
    }

    println!("============================================");
    println!();

    Ok(())
}

fn dump_graph2_param_block(
    process: HANDLE,
    controller: usize,
    offset: usize,
    name: &str,
    client_base: usize,
    client_size: usize,
) -> Result<(), String> {
    let bytes = read_bytes(process, controller + offset, 0x18)?;

    let q0 = u64::from_le_bytes(bytes[0x00..0x08].try_into().unwrap()) as usize;

    let q1 = u64::from_le_bytes(bytes[0x08..0x10].try_into().unwrap()) as usize;

    let q2 = u64::from_le_bytes(bytes[0x10..0x18].try_into().unwrap());

    println!("[GRAPH2] {name} @ +0x{offset:X}");
    println!("         qword = {q0:016X} {q1:016X} {q2:016X}");

    /*
     * Hypothèse :
     * q0 = nom/symbole du paramètre.
     */
    let q0_string = read_c_string_lossy(process, q0, 96);

    println!("         q0 str = \"{q0_string}\"");

    /*
     * Hypothèse :
     * q1 = objet/contexte de bindings AnimGraph2.
     *
     * On essaye son RTTI sans rien écrire.
     */
    let q1_vtable = read_value::<usize>(process, q1).unwrap_or(0);

    println!("         q1 vtable = 0x{q1_vtable:X}");

    match module_containing_address(process, q1_vtable)? {
        Some((module_name, module_base, module_size)) => {
            println!(
                "         q1 module = {} \
                base=0x{:X} size=0x{:X}",
                module_name, module_base, module_size,
            );

            let q1_rtti = rtti_name_from_object(process, module_base, module_size, q1);

            println!("         q1 RTTI = {q1_rtti}");
        }

        None => {
            println!("         q1 module = <aucun module trouvé>");
        }
    }

    /*
     * Décompose q2 pour voir clairement
     * les flags / index.
     */
    println!(
        "         q2 low16=0x{:04X} \
         mid16=0x{:04X} \
         high32=0x{:08X}",
        q2 & 0xFFFF,
        (q2 >> 16) & 0xFFFF,
        q2 >> 32,
    );

    Ok(())
}

fn dump_player_graph2_once(
    process: HANDLE,
    prediction: usize,
    client_base: usize,
    client_size: usize,
) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    let manager = pawn + GRAPH_CONTROLLER_MANAGER;

    let count = read_value::<u64>(process, manager + 0x00)?;
    let data = read_value::<usize>(process, manager + 0x08)?;

    if count == 0 || count > 64 || data < 0x10000 {
        return Err("Graph controllers vector invalide".to_string());
    }

    let mut graph2 = None;

    for index in 0..count as usize {
        let controller = read_value::<usize>(process, data + index * size_of::<usize>())?;

        let name = rtti_name_from_object(process, client_base, client_size, controller);

        if name.contains("CCitadelPlayerPawn_GraphController2") {
            graph2 = Some(controller);
            break;
        }
    }

    let graph2 =
        graph2.ok_or_else(|| "CCitadelPlayerPawn_GraphController2 introuvable".to_string())?;

    println!();
    println!("========== PLAYER GRAPH2 PARAMS ==========");
    println!("[GRAPH2] controller = 0x{graph2:X}");

    dump_graph2_param_block(
        process,
        graph2,
        0xF0,
        "m_flLookHeading",
        client_base,
        client_size,
    )?;

    dump_graph2_param_block(
        process,
        graph2,
        0x108,
        "m_flLookPitch",
        client_base,
        client_size,
    )?;

    dump_graph2_param_block(
        process,
        graph2,
        0x180,
        "m_vLocomotionFacing",
        client_base,
        client_size,
    )?;

    dump_graph2_param_block(
        process,
        graph2,
        0x198,
        "m_vLookTarget",
        client_base,
        client_size,
    )?;

    println!("==========================================");
    println!();

    Ok(())
}

fn find_graph2_context_in_anim_controller(
    process: HANDLE,
    prediction: usize,
    client_base: usize,
    client_size: usize,
) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    /*
     * D'abord on retrouve GraphController2,
     * exactement comme dans notre probe précédent.
     */
    let manager = pawn + GRAPH_CONTROLLER_MANAGER;

    let count = read_value::<u64>(process, manager + 0x00)?;
    let data = read_value::<usize>(process, manager + 0x08)?;

    if count == 0 || count > 64 || data < 0x10000 {
        return Err("Graph controllers vector invalide".to_string());
    }

    let mut graph2 = None;

    for index in 0..count as usize {
        let controller = read_value::<usize>(process, data + index * size_of::<usize>())?;

        let name = rtti_name_from_object(process, client_base, client_size, controller);

        if name.contains("CCitadelPlayerPawn_GraphController2") {
            graph2 = Some(controller);
            break;
        }
    }

    let graph2 =
        graph2.ok_or_else(|| "CCitadelPlayerPawn_GraphController2 introuvable".to_string())?;

    /*
     * q1 du ParamRef look_heading.
     */
    let q1 = read_value::<usize>(process, graph2 + 0xF0 + 0x08)?;

    /*
     * Pawn -> CBodyComponent*
     */
    let body = read_value::<usize>(process, pawn + BODY_COMPONENT)?;

    if body < 0x10000 {
        return Err("CBodyComponent invalide".to_string());
    }

    /*
     * CBodyComponentBaseAnimGraph::m_animationController
     * est embedded, donc pas de dereference ici.
     */
    let anim_controller = body + BODY_ANIM_CONTROLLER;

    println!();
    println!("========== GRAPH2 CONTEXT SEARCH ==========");
    println!("[CTX] pawn            = 0x{pawn:X}");
    println!("[CTX] graph2          = 0x{graph2:X}");
    println!("[CTX] q1              = 0x{q1:X}");
    println!("[CTX] body            = 0x{body:X}");
    println!("[CTX] anim controller = 0x{anim_controller:X}");

    /*
     * CBaseAnimGraphController client est assez gros.
     * On limite volontairement à 0x2000 octets.
     */
    let bytes = read_bytes(process, anim_controller, 0x2000)?;

    let needle = (q1 as u64).to_le_bytes();

    let mut matches = Vec::new();

    for offset in 0..=bytes.len().saturating_sub(8) {
        if bytes[offset..offset + 8] == needle {
            matches.push(offset);
        }
    }

    println!(
        "[CTX] q1 trouvé {} fois dans CBaseAnimGraphController",
        matches.len()
    );

    for offset in matches {
        println!();
        println!("[CTX] MATCH à controller+0x{offset:X}");

        /*
         * Affiche 0x20 octets avant/après pour avoir
         * le contexte structurel, sans rien écrire.
         */
        let start = offset.saturating_sub(0x20);
        let end = (offset + 0x28).min(bytes.len());

        let mut cursor = start & !7usize;

        while cursor + 8 <= end {
            let value = u64::from_le_bytes(bytes[cursor..cursor + 8].try_into().unwrap());

            let marker = if cursor == offset { " <== q1" } else { "" };

            println!("      +0x{cursor:04X} = 0x{value:016X}{marker}");

            cursor += 8;
        }
    }

    println!("===========================================");
    println!();

    Ok(())
}

fn wrap_angle(mut value: f32) -> f32 {
    while value > 180.0 {
        value -= 360.0;
    }

    while value < -180.0 {
        value += 360.0;
    }

    value
}

fn angle_distance(a: f32, b: f32) -> f32 {
    wrap_angle(a - b).abs()
}

fn compare_graph_instance_once(
    process: HANDLE,
    prediction: usize,
    baseline: &mut Option<GraphInstanceBaseline>,
) -> Result<(), String> {
    let pawn = read_value::<usize>(process, prediction + LOCAL_PAWN_IN_PREDICTION)?;

    if pawn < 0x10000 {
        return Err("Local pawn nul".to_string());
    }

    let body = read_value::<usize>(process, pawn + BODY_COMPONENT)?;

    if body < 0x10000 {
        return Err("CBodyComponent invalide".to_string());
    }

    let anim_controller = body + BODY_ANIM_CONTROLLER;

    /*
     * CBaseAnimGraphController::m_pGraphInstanceAG2
     * runtime actuel.
     */
    let instance = read_value::<usize>(process, anim_controller + 0x18E0)?;

    if instance < 0x10000 {
        return Err("m_pGraphInstanceAG2 invalide".to_string());
    }

    let scene_node = read_value::<usize>(process, pawn + SCENE_NODE)?;

    if scene_node < 0x10000 {
        return Err("SceneNode invalide".to_string());
    }

    let scene = read_value::<Vec3>(process, scene_node + SCENE_LOCAL_ANGLES)?;

    let camera = read_value::<Vec3>(process, pawn + CLIENT_CAMERA_ANGLES)?;

    /*
     * Taille connue de CNmGraphInstance :
     * 976 = 0x3D0 octets.
     */
    let bytes = read_bytes(process, instance, 0x3D0)?;

    let relative_yaw = wrap_angle(camera.y - scene.y);

    if baseline.is_none() {
        println!();
        println!("========== GRAPH INSTANCE BASELINE ==========");
        println!("[AG2] instance   = 0x{instance:X}");
        println!("[AG2] scene yaw  = {:.3}", scene.y);
        println!("[AG2] camera yaw = {:.3}", camera.y);
        println!("[AG2] relative   = {:.3}", relative_yaw);
        println!("[AG2] Tourne rapidement la caméra de ~90° puis F11 immédiatement.");
        println!("=============================================");
        println!();

        *baseline = Some(GraphInstanceBaseline {
            instance,
            scene_yaw: scene.y,
            camera_yaw: camera.y,
            bytes,
        });

        return Ok(());
    }

    let first = baseline
        .take()
        .ok_or_else(|| "Baseline AG2 absente".to_string())?;

    if first.instance != instance {
        return Err("CNmGraphInstance a changé entre les deux captures".to_string());
    }

    let first_relative = wrap_angle(first.camera_yaw - first.scene_yaw);

    let camera_delta = wrap_angle(camera.y - first.camera_yaw);

    let scene_delta = wrap_angle(scene.y - first.scene_yaw);

    println!();
    println!("========== GRAPH INSTANCE COMPARE ==========");
    println!("[AG2] instance = 0x{instance:X}");
    println!();
    println!(
        "[AG2] Scene  : {:.3} -> {:.3} | delta={:.3}",
        first.scene_yaw, scene.y, scene_delta,
    );
    println!(
        "[AG2] Camera : {:.3} -> {:.3} | delta={:.3}",
        first.camera_yaw, camera.y, camera_delta,
    );
    println!(
        "[AG2] Relative: {:.3} -> {:.3}",
        first_relative, relative_yaw,
    );

    println!();
    println!("[AG2] candidats angle-like :");

    let mut found = 0usize;

    for offset in (0..0x3D0 - 4).step_by(4) {
        let before = f32::from_le_bytes(first.bytes[offset..offset + 4].try_into().unwrap());

        let after = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());

        if !before.is_finite() || !after.is_finite() {
            continue;
        }

        /*
         * On limite volontairement aux valeurs
         * plausibles pour un angle.
         */
        if before.abs() > 720.0 || after.abs() > 720.0 {
            continue;
        }

        let matches_camera = angle_distance(before, first.camera_yaw) <= 3.0
            && angle_distance(after, camera.y) <= 3.0;

        let matches_scene =
            angle_distance(before, first.scene_yaw) <= 3.0 && angle_distance(after, scene.y) <= 3.0;

        let matches_relative = angle_distance(before, first_relative) <= 3.0
            && angle_distance(after, relative_yaw) <= 3.0;

        /*
         * On teste aussi l'hypothèse radians.
         */
        let matches_camera_rad = (before - first.camera_yaw.to_radians()).abs() <= 0.05
            && (after - camera.y.to_radians()).abs() <= 0.05;

        let matches_scene_rad = (before - first.scene_yaw.to_radians()).abs() <= 0.05
            && (after - scene.y.to_radians()).abs() <= 0.05;

        let matches_relative_rad = (before - first_relative.to_radians()).abs() <= 0.05
            && (after - relative_yaw.to_radians()).abs() <= 0.05;

        if !matches_camera
            && !matches_scene
            && !matches_relative
            && !matches_camera_rad
            && !matches_scene_rad
            && !matches_relative_rad
        {
            continue;
        }

        let mut tags = Vec::new();

        if matches_camera {
            tags.push("CAM_DEG");
        }

        if matches_scene {
            tags.push("SCENE_DEG");
        }

        if matches_relative {
            tags.push("REL_DEG");
        }

        if matches_camera_rad {
            tags.push("CAM_RAD");
        }

        if matches_scene_rad {
            tags.push("SCENE_RAD");
        }

        if matches_relative_rad {
            tags.push("REL_RAD");
        }

        println!(
            "[AG2 CANDIDATE] +0x{offset:03X} : \
             {before:.6} -> {after:.6} | {}",
            tags.join(","),
        );

        found += 1;
    }

    println!();
    println!("[AG2] {found} candidat(s)");
    println!("============================================");
    println!();

    Ok(())
}

fn capture(
    process: HANDLE,
    prediction: usize,
    entity_list: usize,
    client_base: usize,
    client_size: usize,
) {
    println!();
    println!("========== CAPTURE START ==========");
    println!("F11 puis ENTER sur load_slot_4.");

    let started = Instant::now();

    /*
    if let Err(error) = dump_ability_handles(process, prediction, entity_list) {
        eprintln!("[ability] ERROR: {error}");
    }
    */

    let mut samples = Vec::new();

    let mut f11_was_down = false;

    let mut yaw_scan_baseline: Option<YawScanBaseline> = None;

    let mut graph_instance_baseline: Option<GraphInstanceBaseline> = None;

    let mut clear_ground_active = false;
    let mut clear_ground_started: Option<Instant> = None;
    let mut clear_ground_start_position: Option<Vec3> = None;

    while started.elapsed() < CAPTURE_DURATION {
        let f11_down = (unsafe { GetAsyncKeyState(VK_F11 as i32) } as u16 & 0x8000) != 0;

        /*
         * F11 arme le test.
         *
         * Pendant une courte fenêtre, on force uniquement :
         *
         *   m_hGroundEntity = INVALID
         *   m_vecSupport     = 0,0,0
         *
         * Dès qu'un gros changement de position est détecté
         * (le TP), on arrête immédiatement de forcer ces valeurs.
         */
        if f11_down && !f11_was_down {
            if let Err(error) =
                compare_graph_instance_once(process, prediction, &mut graph_instance_baseline)
            {
                eprintln!("[AG2] ERROR: {error}");
            }
        }

        f11_was_down = f11_down;

        if clear_ground_active {
            if let Err(error) = clear_last_velocity_once(process, prediction) {
                eprintln!("[probe] ground clear error: {error}");

                clear_ground_active = false;
            }
        }

        match read_sample(process, prediction, started) {
            Ok(sample) => {
                if clear_ground_active {
                    if let Some(initial) = clear_ground_start_position {
                        let dx = sample.position.x - initial.x;
                        let dy = sample.position.y - initial.y;
                        let dz = sample.position.z - initial.z;

                        let distance_squared = dx * dx + dy * dy + dz * dz;

                        /*
                         * Le slot 4 est à plusieurs milliers
                         * d'unités du toit.
                         *
                         * 500 unités suffit donc largement
                         * à distinguer le TP d'un mouvement normal.
                         */
                        if distance_squared > 500.0 * 500.0 {
                            println!(
                                "[probe] TP détecté à {} ms : \
                                maintien du ground clear pendant 100 ms.",
                                sample.ms,
                            );

                            let post_tp_started = Instant::now();

                            while post_tp_started.elapsed() < Duration::from_millis(100) {
                                if let Err(error) = clear_last_velocity_once(process, prediction) {
                                    eprintln!("[probe] post-TP ground clear error: {error}");

                                    break;
                                }

                                thread::sleep(Duration::from_millis(1));
                            }

                            println!("[probe] fin du post-TP ground clear.");

                            clear_ground_active = false;
                        }
                    }

                    if clear_ground_active {
                        let timed_out = clear_ground_started
                            .map(|instant| instant.elapsed() > Duration::from_millis(1200))
                            .unwrap_or(false);

                        if timed_out {
                            println!(
                                "[probe] ground clear timeout : \
                                 aucun TP détecté."
                            );

                            clear_ground_active = false;
                        }
                    }
                }

                samples.push(sample);
            }

            Err(error) => {
                eprintln!("[probe] sample error: {error}");
            }
        }

        thread::sleep(SAMPLE_DELAY);
    }

    println!("========== CAPTURE END ============");

    println!("[probe] {} samples capturés", samples.len());

    println!();

    if samples.is_empty() {
        return;
    }

    /*
     * Trouve le premier gros déplacement :
     * c'est notre setabsorigin vers le slot 4.
     */
    let initial = samples[0].position;

    let tp_index = samples.iter().position(|sample| {
        let dx = sample.position.x - initial.x;
        let dy = sample.position.y - initial.y;
        let dz = sample.position.z - initial.z;

        dx * dx + dy * dy + dz * dz > 500.0 * 500.0
    });

    if let Some(tp_index) = tp_index {
        println!();
        println!("========== TP HIGH RES ==========");

        /*
         * ~20 samples avant et ~50 après.
         * Comme on sample autour de 1 ms,
         * on verra précisément l'ordre des événements.
         */
        let start = tp_index.saturating_sub(20);
        let end = (tp_index + 50).min(samples.len());

        for sample in &samples[start..end] {
            println!(
                "{:>4}ms | \
                XYZ={:.3},{:.3},{:.3} | \
                yaw={:>8.3} cam={:>8.3} rel={:>8.3} | \
                ground={} hGround=0x{:08X} | \
                absV={:.2},{:.2},{:.2} | \
                srvV={:.2},{:.2},{:.2} | \
                netV={:.2},{:.2},{:.2} | \
                pogo={:.2},{:.2},{:.2} | \
                delta={:.2},{:.2},{:.2} | \
                fall={:.2} | \
                support={:.2},{:.2},{:.2} | \
                move={}/{} coll={} landed={}",
                sample.ms,
                sample.position.x,
                sample.position.y,
                sample.position.z,
                sample.scene_yaw,
                sample.camera_yaw,
                wrap_angle(sample.camera_yaw - sample.scene_yaw),
                sample.on_ground as u8,
                sample.ground_entity,
                sample.velocity.x,
                sample.velocity.y,
                sample.velocity.z,
                sample.server_velocity.x,
                sample.server_velocity.y,
                sample.server_velocity.z,
                sample.network_velocity.x,
                sample.network_velocity.y,
                sample.network_velocity.z,
                sample.pogo_velocity.x,
                sample.pogo_velocity.y,
                sample.pogo_velocity.z,
                sample.position_delta_velocity.x,
                sample.position_delta_velocity.y,
                sample.position_delta_velocity.z,
                sample.fall_velocity,
                sample.support.x,
                sample.support.y,
                sample.support.z,
                sample.move_type,
                sample.actual_move_type,
                sample.colliding as u8,
                sample.landed_on_ground as u8,
            );
        }

        println!("=================================");
    } else {
        println!("[probe] Aucun TP détecté");
    }

    println!();
    println!("F12 = nouvelle capture");
    println!();
}

fn run() -> Result<(), String> {
    let pid = deadlock_pid().ok_or_else(|| "Deadlock n'est pas lancé".to_string())?;

    println!("[probe] Deadlock PID = {pid}");

    let process = open_deadlock(pid)?;

    let (client_base, client_size) = client_module(pid)?;

    println!("[probe] client.dll = 0x{client_base:X} ({client_size} bytes)");

    let prediction = resolve_prediction(process.0, client_base, client_size)?;

    let entity_list = resolve_entity_list(process.0, client_base, client_size)?;

    println!();
    println!("=== MOVEMENT / GROUND PROBE ===");

    println!("F12 puis immédiatement le Load du slot qui bug.");

    println!("Le probe capture 1,4 seconde puis affiche les résultats.");

    println!("Ctrl+C = quitter");

    println!();

    let mut was_down = false;

    loop {
        let down = (unsafe { GetAsyncKeyState(VK_F12 as i32) } as u16 & 0x8000) != 0;

        if down && !was_down {
            capture(process.0, prediction, entity_list, client_base, client_size);
        }

        was_down = down;

        thread::sleep(Duration::from_millis(10));
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("[probe] ERROR: {error}");

        std::process::exit(1);
    }
}
