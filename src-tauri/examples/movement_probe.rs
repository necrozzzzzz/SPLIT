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
            Debug::ReadProcessMemory,
            ToolHelp::{
                CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W,
                TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
            },
        },
        Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
    },
    UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F12},
};

/*
 * C_BaseEntity
 */
const SCENE_NODE: usize = 0x330;
const COLLISION_PROPERTY: usize = 0x340;

const ENTITY_STATE_BASE: usize = 0x400;

const FLAGS: usize = 0x400;
const ABS_VELOCITY: usize = 0x404;

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
 * CGameSceneNode
 */
const ABS_ORIGIN: usize = 0xC8;

/*
 * CPlayer_MovementServices_Humanoid
 */
const FALL_VELOCITY: usize = 0x244;
const GROUND_NORMAL: usize = 0x248;

/*
 * CCitadelPlayer_MovementServices
 */
const TOGGLE_DUCK_ACTIVE: usize = 0x2A0;
const DUCKED: usize = 0x2A1;

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

const CAPTURE_DURATION: Duration = Duration::from_millis(1400);

const SAMPLE_DELAY: Duration = Duration::from_millis(2);

#[derive(Debug, Clone, Copy)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    ms: u128,

    pawn: usize,

    position: Vec3,

    flags: u32,
    on_ground: bool,

    velocity: Vec3,

    move_type: u8,
    actual_move_type: u8,

    ground_entity: u32,
    ground_body_index: i32,

    gravity_scale: f32,
    gravity_disabled: bool,

    actual_gravity_scale: f32,
    gravity_actually_disabled: bool,

    fall_velocity: f32,

    ground_normal: Vec3,
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
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };

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

    let move_type = entity[MOVE_TYPE - ENTITY_STATE_BASE];

    let actual_move_type = entity[ACTUAL_MOVE_TYPE - ENTITY_STATE_BASE];

    let ground_entity = u32_at(&entity, GROUND_ENTITY - ENTITY_STATE_BASE);

    let ground_body_index = i32_at(&entity, GROUND_BODY_INDEX - ENTITY_STATE_BASE);

    let gravity_scale = f32_at(&entity, GRAVITY_SCALE - ENTITY_STATE_BASE);

    let gravity_disabled = entity[GRAVITY_DISABLED - ENTITY_STATE_BASE] != 0;

    let actual_gravity_scale = f32_at(&entity, ACTUAL_GRAVITY_SCALE - ENTITY_STATE_BASE);

    let gravity_actually_disabled = entity[GRAVITY_ACTUALLY_DISABLED - ENTITY_STATE_BASE] != 0;

    let fall_velocity = f32_at(&movement_state, 0);

    let ground_normal = vec3_at(&movement_state, GROUND_NORMAL - FALL_VELOCITY);

    let support = vec3_at(&movement_state, SUPPORT - FALL_VELOCITY);

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

        flags,
        on_ground: flags & 0x1 != 0,

        velocity,

        move_type,
        actual_move_type,

        ground_entity,
        ground_body_index,

        gravity_scale,
        gravity_disabled,

        actual_gravity_scale,
        gravity_actually_disabled,

        fall_velocity,

        ground_normal,
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
        "{:>4}ms | XYZ={:>8.3},{:>8.3},{:>8.3} velZ={:>8.2} | \
ground={} hGround=0x{:08X} | \
move={}/{} | duck={}/{} | \
hullZ={:.2}..{:.2} | \
capsuleZ={:.2}..{:.2} r={:.2} | \
solid={}/{} physics={} | coll={} landed={}",
        sample.ms,
        sample.position.x,
        sample.position.y,
        sample.position.z,
        sample.velocity.z,
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
}

fn capture(process: HANDLE, prediction: usize) {
    println!();
    println!("========== CAPTURE START ==========");

    println!("Charge ton slot MAINTENANT.");

    let started = Instant::now();

    let mut samples = Vec::new();

    while started.elapsed() < CAPTURE_DURATION {
        match read_sample(process, prediction, started) {
            Ok(sample) => {
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
     * On imprime :
     *
     * - le premier sample ;
     * - environ toutes les 10 ms ;
     * - immédiatement chaque transition
     *   d'état importante.
     *
     * Ainsi on ne spamme pas le terminal
     * pendant le jeu, mais on ne rate pas
     * un NOCLIP/ground transition bref.
     */
    for index in 0..samples.len() {
        let sample = &samples[index];

        let periodic = index == 0 || index % 5 == 0;

        let transition = index > 0 && discrete_changed(&samples[index - 1], sample);

        if periodic || transition {
            print_sample(sample);
        }
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
            capture(process.0, prediction);
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
