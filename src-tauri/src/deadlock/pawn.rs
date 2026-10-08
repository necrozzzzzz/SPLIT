use std::mem::{size_of, zeroed};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        Memory::{VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS},
        Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
    },
};

use super::{camera, interfaces};

const RESOURCE_SERVICE_MODULE: &str = "engine2.dll";
const RESOURCE_SERVICE_INTERFACE: &str = "GameResourceServiceClientV001";
// Source 2 CGameResourceService ABI. Unlike schema fields, this engine-owned
// member is not exposed by SchemaSystem; every pointer reached through it is
// validated before use.
const RESOURCE_SERVICE_ENTITY_SYSTEM_OFFSET: u64 = 0x58;
const ENTITY_LIST_OFFSET: u64 = 0x10;
const ENTITY_CHUNK_COUNT: usize = 64;
const ENTITIES_PER_CHUNK: usize = 512;
const ENTITY_IDENTITY_HANDLE_OFFSET: usize = 0x10;
const ENTITY_INDEX_MASK: u32 = 0x7FFF;
const MAX_ENTITIES: usize = ENTITY_CHUNK_COUNT * ENTITIES_PER_CHUNK;
const MAX_RTTI_NAME: usize = 128;
const MIN_USER_ADDRESS: u64 = 0x1_0000;
const MAX_USER_ADDRESS: u64 = 0x0000_7FFF_FFFF_FFFF;

struct ReadOnlyProcess(HANDLE);

impl Drop for ReadOnlyProcess {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EntityIdentity {
    instance: u64,
    handle: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PawnResolution {
    pub(crate) controller: u64,
    pub(crate) pawn_handle: u32,
    pub(crate) pawn: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PawnTelemetry {
    pub(crate) position: [f32; 3],
    pub(crate) velocity: [f32; 3],
    pub(crate) pitch: Option<f32>,
    pub(crate) yaw: Option<f32>,
    pub(crate) diagnostics: PawnTelemetryDiagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PawnTelemetryDiagnostics {
    pub(crate) scene_node: u64,
    pub(crate) local_origin: [f32; 3],
    pub(crate) absolute_origin: [f32; 3],
    pub(crate) wrapped_local_origin: [f32; 3],
    pub(crate) old_origin: [f32; 3],
    pub(crate) view_angles: [f32; 3],
    pub(crate) client_camera_angles: [f32; 3],
    pub(crate) prediction_error: [f32; 3],
    pub(crate) prediction_error_time: f32,
    pub(crate) simulation_tick: u32,
    pub(crate) simulation_time: f32,
    pub(crate) coherent: bool,
    pub(crate) read_attempts: u8,
}

#[derive(Debug, Clone, Copy)]
struct TelemetrySnapshot {
    scene_node: u64,
    position: [f32; 3],
    absolute_origin: [f32; 3],
    wrapped_local_origin: [f32; 3],
    velocity: [f32; 3],
    old_origin: [f32; 3],
    view_angles: [f32; 3],
    client_camera_angles: [f32; 3],
    prediction_error: [f32; 3],
    prediction_error_time: f32,
    simulation_tick: u32,
    simulation_time: f32,
}

pub(crate) struct PawnTelemetryReader {
    process: ReadOnlyProcess,
    resolution: PawnResolution,
    pawn_handle_offset: u32,
    game_scene_node_offset: u32,
    local_origin_offset: u32,
    absolute_origin_offset: u32,
    wrapped_local_origin_offset: u32,
    absolute_velocity_offset: u32,
    simulation_tick_offset: u32,
    simulation_time_offset: u32,
    old_origin_offset: u32,
    view_angles_offset: u32,
    client_camera_angles_offset: u32,
    prediction_error_offset: u32,
    prediction_error_time_offset: u32,
    pawn_snapshot_start: u32,
    pawn_snapshot_size: usize,
    scene_snapshot_start: u32,
    scene_snapshot_size: usize,
}

impl PawnTelemetryReader {
    pub(crate) fn new(
        pid: u32,
        resolution: PawnResolution,
        route_schema: super::schema::RouteSchema,
    ) -> Result<Self, String> {
        let (pawn_snapshot_start, pawn_snapshot_size) = field_span(&[
            (route_schema.game_scene_node_offset, 8),
            (route_schema.absolute_velocity_offset, 12),
            (route_schema.simulation_tick_offset, 4),
            (route_schema.simulation_time_offset, 4),
            (route_schema.old_origin_offset, 12),
            (route_schema.view_angles_offset, 12),
            (route_schema.client_camera_angles_offset, 12),
            (route_schema.prediction_error_offset, 12),
            (route_schema.prediction_error_time_offset, 4),
        ])?;
        let (scene_snapshot_start, scene_snapshot_size) = field_span(&[
            (route_schema.local_origin_offset, 12),
            (route_schema.absolute_origin_offset, 12),
            (route_schema.wrapped_local_origin_offset, 12),
        ])?;
        let reader = Self {
            process: open_process_read_only(pid)?,
            resolution,
            pawn_handle_offset: route_schema.pawn_handle_offset,
            game_scene_node_offset: route_schema.game_scene_node_offset,
            local_origin_offset: route_schema.local_origin_offset,
            absolute_origin_offset: route_schema.absolute_origin_offset,
            wrapped_local_origin_offset: route_schema.wrapped_local_origin_offset,
            absolute_velocity_offset: route_schema.absolute_velocity_offset,
            simulation_tick_offset: route_schema.simulation_tick_offset,
            simulation_time_offset: route_schema.simulation_time_offset,
            old_origin_offset: route_schema.old_origin_offset,
            view_angles_offset: route_schema.view_angles_offset,
            client_camera_angles_offset: route_schema.client_camera_angles_offset,
            prediction_error_offset: route_schema.prediction_error_offset,
            prediction_error_time_offset: route_schema.prediction_error_time_offset,
            pawn_snapshot_start,
            pawn_snapshot_size,
            scene_snapshot_start,
            scene_snapshot_size,
        };
        reader.ensure_current_pawn()?;
        Ok(reader)
    }

    pub(crate) fn read(&self) -> Result<PawnTelemetry, String> {
        self.ensure_current_pawn()?;
        let first = self.read_snapshot()?;
        let second = self.read_snapshot()?;
        let (snapshot, read_attempts, coherent) = if snapshots_match(&first, &second) {
            (second, 2, true)
        } else {
            let third = self.read_snapshot()?;
            (third, 3, snapshots_match(&second, &third))
        };

        Ok(PawnTelemetry {
            // Of the reactive scene-node sources measured at runtime,
            // m_vecAbsOrigin was the least glitch-prone. The local source stays
            // in diagnostics so the next capture can verify this choice.
            position: snapshot.absolute_origin,
            velocity: snapshot.velocity,
            // m_angClientCamera remained coherent during the observed one-sample
            // v_angle tear. v_angle is retained below as a runtime diagnostic.
            pitch: Some(snapshot.client_camera_angles[0]),
            yaw: Some(snapshot.client_camera_angles[1]),
            diagnostics: PawnTelemetryDiagnostics {
                scene_node: snapshot.scene_node,
                local_origin: snapshot.position,
                absolute_origin: snapshot.absolute_origin,
                wrapped_local_origin: snapshot.wrapped_local_origin,
                old_origin: snapshot.old_origin,
                view_angles: snapshot.view_angles,
                client_camera_angles: snapshot.client_camera_angles,
                prediction_error: snapshot.prediction_error,
                prediction_error_time: snapshot.prediction_error_time,
                simulation_tick: snapshot.simulation_tick,
                simulation_time: snapshot.simulation_time,
                coherent,
                read_attempts,
            },
        })
    }

    fn read_snapshot(&self) -> Result<TelemetrySnapshot, String> {
        let pawn = self.resolution.pawn;
        let pawn_snapshot = read_exact(
            self.process.0,
            checked_add(
                pawn,
                self.pawn_snapshot_start as u64,
                "pawn telemetry snapshot",
            )?,
            self.pawn_snapshot_size,
            "pawn telemetry snapshot",
        )?;
        let scene_node = read_u64(
            &pawn_snapshot,
            (self.game_scene_node_offset - self.pawn_snapshot_start) as usize,
            "game scene node",
        )?;
        let scene_snapshot = read_exact(
            self.process.0,
            checked_add(
                scene_node,
                self.scene_snapshot_start as u64,
                "scene telemetry snapshot",
            )?,
            self.scene_snapshot_size,
            "scene telemetry snapshot",
        )?;
        let pawn_field = |offset: u32, label: &str| {
            read_f32x3(
                &pawn_snapshot,
                (offset - self.pawn_snapshot_start) as usize,
                label,
            )
        };
        let scene_field = |offset: u32, label: &str| {
            read_f32x3(
                &scene_snapshot,
                (offset - self.scene_snapshot_start) as usize,
                label,
            )
        };
        let pawn_u32 = |offset: u32, label: &str| {
            read_u32(
                &pawn_snapshot,
                (offset - self.pawn_snapshot_start) as usize,
                label,
            )
        };
        let pawn_f32 = |offset: u32, label: &str| {
            read_f32(
                &pawn_snapshot,
                (offset - self.pawn_snapshot_start) as usize,
                label,
            )
        };

        Ok(TelemetrySnapshot {
            scene_node,
            position: scene_field(self.local_origin_offset, "local origin")?,
            absolute_origin: scene_field(self.absolute_origin_offset, "absolute origin")?,
            wrapped_local_origin: scene_field(
                self.wrapped_local_origin_offset,
                "wrapped local origin",
            )?,
            velocity: pawn_field(self.absolute_velocity_offset, "absolute velocity")?,
            old_origin: pawn_field(self.old_origin_offset, "old origin")?,
            view_angles: pawn_field(self.view_angles_offset, "view angles")?,
            client_camera_angles: pawn_field(
                self.client_camera_angles_offset,
                "client camera angles",
            )?,
            prediction_error: pawn_field(self.prediction_error_offset, "prediction error")?,
            prediction_error_time: pawn_f32(
                self.prediction_error_time_offset,
                "prediction error time",
            )?,
            simulation_tick: pawn_u32(self.simulation_tick_offset, "simulation tick")?,
            simulation_time: pawn_f32(self.simulation_time_offset, "simulation time")?,
        })
    }

    fn ensure_current_pawn(&self) -> Result<(), String> {
        let current = read_remote_u32(
            self.process.0,
            checked_add(
                self.resolution.controller,
                self.pawn_handle_offset as u64,
                "pawn handle member",
            )?,
            "local pawn handle",
        )?;
        if current != self.resolution.pawn_handle {
            return Err("Local pawn changed during route recording".to_string());
        }
        Ok(())
    }
}

fn snapshots_match(left: &TelemetrySnapshot, right: &TelemetrySnapshot) -> bool {
    left.scene_node == right.scene_node
        && left.simulation_tick == right.simulation_tick
        && left.simulation_time.to_bits() == right.simulation_time.to_bits()
        && f32x3_bits_equal(left.position, right.position)
        && f32x3_bits_equal(left.absolute_origin, right.absolute_origin)
        && f32x3_bits_equal(left.wrapped_local_origin, right.wrapped_local_origin)
        && f32x3_bits_equal(left.velocity, right.velocity)
        && f32x3_bits_equal(left.old_origin, right.old_origin)
        && f32x3_bits_equal(left.view_angles, right.view_angles)
        && f32x3_bits_equal(left.client_camera_angles, right.client_camera_angles)
        && f32x3_bits_equal(left.prediction_error, right.prediction_error)
        && left.prediction_error_time.to_bits() == right.prediction_error_time.to_bits()
}

fn f32x3_bits_equal(left: [f32; 3], right: [f32; 3]) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| left.to_bits() == right.to_bits())
}

fn field_span(fields: &[(u32, usize)]) -> Result<(u32, usize), String> {
    let start = fields
        .iter()
        .map(|(offset, _)| *offset)
        .min()
        .ok_or_else(|| "Telemetry field span is empty".to_string())?;
    let end = fields.iter().try_fold(0_u64, |current, (offset, size)| {
        u64::from(*offset)
            .checked_add(*size as u64)
            .map(|value| current.max(value))
            .ok_or_else(|| "Telemetry field span overflow".to_string())
    })?;
    let size = usize::try_from(end - u64::from(start))
        .map_err(|_| "Telemetry field span does not fit usize".to_string())?;
    Ok((start, size))
}

pub(crate) fn resolve_local_pawn(
    pid: u32,
    local_flag_offset: u32,
    pawn_handle_offset: u32,
    identity_size: u32,
) -> Result<PawnResolution, String> {
    let identity_size = validate_identity_size(identity_size)?;
    let service =
        interfaces::find_interface(pid, RESOURCE_SERVICE_MODULE, RESOURCE_SERVICE_INTERFACE)?
            .ok_or_else(|| format!("Exact interface {RESOURCE_SERVICE_INTERFACE} was not found"))?;
    let process = open_process_read_only(pid)?;
    let (client_base, client_size) = camera::find_module(pid, "client.dll")?;
    let client_range = (client_base as u64, client_size as u64);
    let (engine_base, engine_size) = camera::find_module(pid, RESOURCE_SERVICE_MODULE)?;
    let engine_range = (engine_base as u64, engine_size as u64);

    validate_object_vtable(process.0, service, None, engine_range, "resource service")?;
    let entity_system = read_remote_u64(
        process.0,
        checked_add(
            service,
            RESOURCE_SERVICE_ENTITY_SYSTEM_OFFSET,
            "entity-system member",
        )?,
        "client entity system",
    )?;
    validate_object_vtable(
        process.0,
        entity_system,
        None,
        client_range,
        "client entity system",
    )?;

    let chunks = read_chunks(process.0, entity_system)?;
    let mut visited = 0usize;
    let controller = 'find_controller: {
        for (chunk_index, chunk) in chunks.iter().copied().enumerate() {
            if chunk == 0 {
                continue;
            }
            let identities = read_exact(
                process.0,
                chunk,
                identity_size * ENTITIES_PER_CHUNK,
                "entity identity chunk",
            )?;
            for slot in 0..ENTITIES_PER_CHUNK {
                visited = visited
                    .checked_add(1)
                    .ok_or_else(|| "Entity traversal overflow".to_string())?;
                if visited > MAX_ENTITIES {
                    return Err("Entity traversal limit exceeded".to_string());
                }
                let index = chunk_index * ENTITIES_PER_CHUNK + slot;
                let identity = identity_from_chunk(&identities, slot, identity_size)?;
                if identity.instance == 0 || identity_index(identity.handle) != Some(index) {
                    continue;
                }
                let flag_address = checked_add(
                    identity.instance,
                    local_flag_offset as u64,
                    "local-controller flag",
                )?;
                let Ok(flag_bytes) =
                    read_exact(process.0, flag_address, 1, "local-controller flag")
                else {
                    continue;
                };
                if !is_local_controller_flag(flag_bytes[0]) {
                    continue;
                }
                if validate_object_vtable(
                    process.0,
                    identity.instance,
                    Some("PlayerController"),
                    client_range,
                    "local player controller",
                )
                .is_err()
                {
                    continue;
                }
                break 'find_controller identity.instance;
            }
        }
        return Err("No validated local player controller was found".to_string());
    };
    #[cfg(debug_assertions)]
    super::district::debug_controller_found(controller, visited);
    let pawn_handle = read_remote_u32(
        process.0,
        checked_add(controller, pawn_handle_offset as u64, "pawn handle member")?,
        "local pawn handle",
    )?;
    let pawn = resolve_handle(process.0, &chunks, pawn_handle, identity_size)?;
    validate_object_vtable(
        process.0,
        pawn,
        Some("C_CitadelPlayerPawn"),
        client_range,
        "local Citadel player pawn",
    )?;
    Ok(PawnResolution {
        controller,
        pawn_handle,
        pawn,
    })
}

pub(crate) fn resolve_first_other_citadel_pawn(
    pid: u32,
    identity_size: u32,
    local_pawn: u64,
) -> Result<u64, String> {
    let identity_size = validate_identity_size(identity_size)?;

    let service =
        interfaces::find_interface(pid, RESOURCE_SERVICE_MODULE, RESOURCE_SERVICE_INTERFACE)?
            .ok_or_else(|| format!("Exact interface {RESOURCE_SERVICE_INTERFACE} was not found"))?;

    let process = open_process_read_only(pid)?;
    let (client_base, client_size) = camera::find_module(pid, "client.dll")?;
    let client_range = (client_base as u64, client_size as u64);

    let entity_system = read_remote_u64(
        process.0,
        checked_add(
            service,
            RESOURCE_SERVICE_ENTITY_SYSTEM_OFFSET,
            "entity-system member",
        )?,
        "client entity system",
    )?;

    let chunks = read_chunks(process.0, entity_system)?;

    for (chunk_index, chunk) in chunks.iter().copied().enumerate() {
        if chunk == 0 {
            continue;
        }

        let identities = read_exact(
            process.0,
            chunk,
            identity_size * ENTITIES_PER_CHUNK,
            "entity identity chunk",
        )?;

        for slot in 0..ENTITIES_PER_CHUNK {
            let index = chunk_index * ENTITIES_PER_CHUNK + slot;
            let identity = identity_from_chunk(&identities, slot, identity_size)?;

            if identity.instance == 0 {
                continue;
            }

            if identity_index(identity.handle) != Some(index) {
                continue;
            }

            if identity.instance == local_pawn {
                continue;
            }

            if validate_object_vtable(
                process.0,
                identity.instance,
                Some("C_CitadelPlayerPawn"),
                client_range,
                "other Citadel player pawn",
            )
            .is_ok()
            {
                return Ok(identity.instance);
            }
        }
    }

    Err("No other C_CitadelPlayerPawn was found".to_string())
}


pub(crate) fn resolve_entity_by_index(
    pid: u32,
    entity_index: u32,
    identity_size: u32,
) -> Result<u64, String> {
    let identity_size = validate_identity_size(identity_size)?;

    let service =
        interfaces::find_interface(pid, RESOURCE_SERVICE_MODULE, RESOURCE_SERVICE_INTERFACE)?
            .ok_or_else(|| format!("Exact interface {RESOURCE_SERVICE_INTERFACE} was not found"))?;

    let process = open_process_read_only(pid)?;

    let entity_system = read_remote_u64(
        process.0,
        checked_add(
            service,
            RESOURCE_SERVICE_ENTITY_SYSTEM_OFFSET,
            "entity-system member",
        )?,
        "client entity system",
    )?;

    let chunks = read_chunks(process.0, entity_system)?;

    let index = entity_index as usize;
    let chunk_index = validate_entity_index(index, chunks.len())?;
    let slot = index % ENTITIES_PER_CHUNK;

    let chunk = *chunks
        .get(chunk_index)
        .ok_or_else(|| format!("Entity index {index} exceeds entity chunks"))?;

    if chunk == 0 {
        return Err(format!("Entity index {index} points to a null chunk"));
    }

    let identity_address =
        checked_add(chunk, (slot * identity_size) as u64, "entity identity")?;

    let bytes = read_exact(
        process.0,
        identity_address,
        identity_size,
        "entity identity",
    )?;

    let identity = identity_from_chunk(&bytes, 0, identity_size)?;

    if identity.instance == 0 {
        return Err(format!("Entity index {index} has a null instance"));
    }

    if identity_index(identity.handle) != Some(index) {
        return Err(format!(
            "Entity index mismatch: requested {index}, identity handle=0x{:08X}",
            identity.handle
        ));
    }

    Ok(identity.instance)
}

pub(crate) fn log_local_pawn(
    pid: u32,
    local_flag_offset: u32,
    pawn_handle_offset: u32,
    identity_size: u32,
) -> Result<PawnResolution, String> {
    let resolution = resolve_local_pawn(pid, local_flag_offset, pawn_handle_offset, identity_size)?;
    println!(
        "[SPLIT][Pawn] local controller = 0x{:016X}",
        resolution.controller
    );
    println!(
        "[SPLIT][Pawn] local pawn handle = 0x{:08X}",
        resolution.pawn_handle
    );
    println!("[SPLIT][Pawn] local pawn = 0x{:016X}", resolution.pawn);
    Ok(resolution)
}

pub(crate) fn read_current_pawn_handle(
    pid: u32,
    controller: u64,
    pawn_handle_offset: u32,
) -> Result<u32, String> {
    let process = open_process_read_only(pid)?;
    read_remote_u32(
        process.0,
        checked_add(controller, pawn_handle_offset as u64, "pawn handle member")?,
        "local pawn handle",
    )
}

pub(crate) fn debug_bot_movement(
    pid: u32,
    entity_index: u32,
    identity_size: u32,
    movement_services_offset: u32,
) -> Result<(), String> {
    let process = open_process_read_only(pid)?;
    let pawn = resolve_entity_by_index(pid, entity_index, identity_size)?;

    let movement_services = read_remote_u64(
        process.0,
        checked_add(
            pawn,
            movement_services_offset as u64,
            "movement services pointer",
        )?,
        "movement services",
    )?;

    if movement_services == 0 {
        return Err("Movement services pointer is null".to_string());
    }

    let cmd_forward = read_remote_f32(
        process.0,
        movement_services + 0x1A0,
        "m_flCmdForwardMove",
    )?;

    let cmd_left = read_remote_f32(
        process.0,
        movement_services + 0x1A4,
        "m_flCmdLeftMove",
    )?;

    let forward = read_remote_f32(
        process.0,
        movement_services + 0x1C0,
        "m_flForwardMove",
    )?;

    let left = read_remote_f32(
        process.0,
        movement_services + 0x1C4,
        "m_flLeftMove",
    )?;

    println!(
        "[SPLIT][BotMove] ent={} pawn=0x{:X} move=0x{:X} cmd_fwd={:.3} cmd_left={:.3} fwd={:.3} left={:.3}",
        entity_index,
        pawn,
        movement_services,
        cmd_forward,
        cmd_left,
        forward,
        left
    );

    Ok(())
}

pub(crate) fn debug_bot_movement_from_pawn(
    pid: u32,
    pawn: u64,
    movement_services_offset: u32,
) -> Result<(), String> {
    let process = open_process_read_only(pid)?;

    let movement_services = read_remote_u64(
        process.0,
        checked_add(
            pawn,
            movement_services_offset as u64,
            "movement services pointer",
        )?,
        "movement services",
    )?;

    if movement_services == 0 {
        return Err("Movement services pointer is null".to_string());
    }

    let abs_velocity = read_remote_f32x3(
        process.0,
        pawn + 0x400,
        "m_vecAbsVelocity",
    )?;

    let last_impulses = read_remote_f32x3(
        process.0,
        movement_services + 0x1CC,
        "m_vecLastMovementImpulses",
    )?;

    let position_delta_velocity = read_remote_f32x3(
        process.0,
        movement_services + 0x288,
        "m_vPositionDeltaVelocity",
    )?;

    let last_wish_dir = read_remote_f32x3(
        process.0,
        movement_services + 0x2F4,
        "m_vLastWishDir",
    )?;

    let input_commitment = read_remote_f32(
        process.0,
        movement_services + 0x2E8,
        "m_flInputDirectionCommitment",
    )?;

    let ag2_turn_speed = read_remote_f32(
        process.0,
        movement_services + 0x2E0,
        "m_flAG2TurnSpeed",
    )?;

    println!(
        "[SPLIT][BotMove] pawn=0x{:X} move=0x{:X} \
vel=({:.3},{:.3},{:.3}) \
imp=({:.3},{:.3},{:.3}) \
delta=({:.3},{:.3},{:.3}) \
wish=({:.3},{:.3},{:.3}) \
commit={:.3} ag2turn={:.3}",
        pawn,
        movement_services,
        abs_velocity[0],
        abs_velocity[1],
        abs_velocity[2],
        last_impulses[0],
        last_impulses[1],
        last_impulses[2],
        position_delta_velocity[0],
        position_delta_velocity[1],
        position_delta_velocity[2],
        last_wish_dir[0],
        last_wish_dir[1],
        last_wish_dir[2],
        input_commitment,
        ag2_turn_speed
    );

    Ok(())
}

pub(crate) fn debug_scan_other_pawns(
    pid: u32,
    identity_size: u32,
    local_pawn: u64,
    game_scene_node_offset: u32,
    absolute_origin_offset: u32,
    ang_rotation_offset: u32,
    ang_abs_rotation_offset: u32,
    ang_wrapped_local_rotation_offset: u32,
) -> Result<(), String> {
    let identity_size = validate_identity_size(identity_size)?;

    let service =
        interfaces::find_interface(pid, RESOURCE_SERVICE_MODULE, RESOURCE_SERVICE_INTERFACE)?
            .ok_or_else(|| format!("Exact interface {RESOURCE_SERVICE_INTERFACE} was not found"))?;

    let process = open_process_read_only(pid)?;

    let (client_base, client_size) = camera::find_module(pid, "client.dll")?;
    let client_range = (client_base as u64, client_size as u64);

    let entity_system = read_remote_u64(
        process.0,
        checked_add(
            service,
            RESOURCE_SERVICE_ENTITY_SYSTEM_OFFSET,
            "entity-system member",
        )?,
        "client entity system",
    )?;

    let chunks = read_chunks(process.0, entity_system)?;

    let mut pawns = Vec::new();

    for (chunk_index, chunk) in chunks.iter().copied().enumerate() {
        if chunk == 0 {
            continue;
        }

        let identities = read_exact(
            process.0,
            chunk,
            identity_size * ENTITIES_PER_CHUNK,
            "entity identity chunk",
        )?;

        for slot in 0..ENTITIES_PER_CHUNK {
            let index = chunk_index * ENTITIES_PER_CHUNK + slot;
            let identity = identity_from_chunk(&identities, slot, identity_size)?;

            if identity.instance == 0 {
                continue;
            }

            if identity_index(identity.handle) != Some(index) {
                continue;
            }

            if identity.instance == local_pawn {
                continue;
            }

            if validate_object_vtable(
                process.0,
                identity.instance,
                Some("C_CitadelPlayerPawn"),
                client_range,
                "other Citadel player pawn",
            )
            .is_ok()
            {
                pawns.push((index, identity.instance));
            }
        }
    }
    

    println!(
        "[SPLIT][PawnScan] found {} other Citadel pawns",
        pawns.len()
    );

    for sample in 0..50 {
        for &(entity_index, pawn) in &pawns {


            let controller_handle = read_remote_u32(
                process.0,
                pawn + 0x1050,
                "m_hController",
            )
            .unwrap_or(0);

            let default_controller_handle = read_remote_u32(
                process.0,
                pawn + 0x1054,
                "m_hDefaultController",
            )
            .unwrap_or(0);

        

            let controller_index = identity_index(controller_handle);
            let default_controller_index = identity_index(default_controller_handle);

            let mut controller_address = 0_u64;
            let mut controller_rtti = String::from("unresolved");

            if controller_handle != 0 && controller_handle != u32::MAX {
                match resolve_handle(
                    process.0,
                    &chunks,
                    controller_handle,
                    identity_size,
                ) {
                    Ok(controller) => {
                        controller_address = controller;

                        match read_remote_u64(
                            process.0,
                            controller,
                            "bot controller vtable",
                        ) {
                            Ok(vtable) => {
                                controller_rtti = read_msvc_rtti_name(
                                    process.0,
                                    vtable,
                                    client_range,
                                    "bot controller",
                                )
                                .unwrap_or_else(|error| format!("RTTI_ERROR:{error}"));
                            }

                            Err(error) => {
                                controller_rtti = format!("VTABLE_ERROR:{error}");
                            }
                        }
                    }

                    Err(error) => {
                        controller_rtti = format!("RESOLVE_ERROR:{error}");
                    }
                }
            }

            let mut default_controller_address = 0_u64;
            let mut default_controller_rtti = String::from("unresolved");

            if default_controller_handle != 0
                && default_controller_handle != u32::MAX
            {
                match resolve_handle(
                    process.0,
                    &chunks,
                    default_controller_handle,
                    identity_size,
                ) {
                    Ok(controller) => {
                        default_controller_address = controller;

                        match read_remote_u64(
                            process.0,
                            controller,
                            "default bot controller vtable",
                        ) {
                            Ok(vtable) => {
                                default_controller_rtti = read_msvc_rtti_name(
                                    process.0,
                                    vtable,
                                    client_range,
                                    "default bot controller",
                                )
                                .unwrap_or_else(|error| format!("RTTI_ERROR:{error}"));
                            }

                            Err(error) => {
                                default_controller_rtti =
                                    format!("VTABLE_ERROR:{error}");
                            }
                        }
                    }

                    Err(error) => {
                        default_controller_rtti =
                            format!("RESOLVE_ERROR:{error}");
                    }
                }
            }


            let body_component = read_remote_u64(
                process.0,
                pawn + 0x30,
                "m_CBodyComponent",
            )
            .unwrap_or(0);

            let mut body_rtti = String::from("null");

            if body_component != 0 {
                match read_remote_u64(
                    process.0,
                    body_component,
                    "body component vtable",
                ) {
                    Ok(vtable) => {
                        body_rtti = read_msvc_rtti_name(
                            process.0,
                            vtable,
                            client_range,
                            "body component",
                        )
                        .unwrap_or_else(|error| format!("RTTI_ERROR:{error}"));
                    }

                    Err(error) => {
                        body_rtti = format!("VTABLE_ERROR:{error}");
                    }
                }
            }

            let animation_controller = if body_component != 0 {
                body_component + 0x530
            } else {
                0
            };

            let anim_ctrl_qword0 = read_remote_u64(
                process.0,
                animation_controller,
                "animation controller qword0",
            )
            .unwrap_or(0);

            let anim_ctrl_qword1 = read_remote_u64(
                process.0,
                animation_controller + 0x8,
                "animation controller qword1",
            )
            .unwrap_or(0);

            let anim_ctrl_qword2 = read_remote_u64(
                process.0,
                animation_controller + 0x10,
                "animation controller qword2",
            )
            .unwrap_or(0);

            let anim_ctrl_qword3 = read_remote_u64(
                process.0,
                animation_controller + 0x18,
                "animation controller qword3",
            )
            .unwrap_or(0);

            if sample == 0 {
                println!(
                    "[SPLIT][AnimCtrlScan] base=0x{:X}",
                    animation_controller,
                );

                for offset in (0x2E0_u64..=0x3D0_u64).step_by(8) {
                    let value = read_remote_u64(
                        process.0,
                        animation_controller + offset,
                        "animation controller scan qword",
                    )
                    .unwrap_or(0);

                    if value >= client_range.0
                        && value < client_range.0 + client_range.1
                    {
                        let rtti = read_msvc_rtti_name(
                            process.0,
                            value,
                            client_range,
                            "animation controller scan",
                        )
                        .unwrap_or_else(|error| format!("RTTI_ERROR:{error}"));

                        println!(
                            "[SPLIT][AnimCtrlScan] +0x{:03X} value=0x{:016X} RTTI={:?}",
                            offset,
                            value,
                            rtti,
                        );
                    } else {
                        println!(
                            "[SPLIT][AnimCtrlScan] +0x{:03X} value=0x{:016X}",
                            offset,
                            value,
                        );
                    }
                }


                let ag2_candidate = read_remote_u64(
                    process.0,
                    animation_controller + 0x3D0,
                    "AG2 candidate +0x3D0",
                )
                .unwrap_or(0);

                let mut ag2_candidate_rtti = String::from("null");

                if ag2_candidate != 0 {
                    match read_remote_u64(
                        process.0,
                        ag2_candidate,
                        "AG2 candidate vtable",
                    ) {
                        Ok(vtable) => {
                            ag2_candidate_rtti = read_msvc_rtti_name(
                                process.0,
                                vtable,
                                client_range,
                                "AG2 candidate",
                            )
                            .unwrap_or_else(|error| format!("RTTI_ERROR:{error}"));
                        }

                        Err(error) => {
                            ag2_candidate_rtti =
                                format!("VTABLE_ERROR:{error}");
                        }
                    }
                }

                println!(
                    "[SPLIT][AG2Candidate] ptr=0x{:X} RTTI={:?}",
                    ag2_candidate,
                    ag2_candidate_rtti,
                );

            }

            if sample == 0 {
                println!(
                    "[SPLIT][PawnScan] sample={} ent={} pawn=0x{:X} body=0x{:X} animController=0x{:X}",
                    sample,
                    entity_index,
                    pawn,
                    body_component,
                    animation_controller,
                );
            }

            let mut q1_rtti = String::from("null");

            if anim_ctrl_qword1 != 0 {
                match read_remote_u64(
                    process.0,
                    anim_ctrl_qword1,
                    "animation controller q1 vtable",
                ) {
                    Ok(vtable) => {
                        q1_rtti = read_msvc_rtti_name(
                            process.0,
                            vtable,
                            client_range,
                            "animation controller q1 object",
                        )
                        .unwrap_or_else(|error| format!("RTTI_ERROR:{error}"));
                    }

                    Err(error) => {
                        q1_rtti = format!("VTABLE_ERROR:{error}");
                    }
                }
            }

            let mut animation_controller_rtti = String::from("null");

            if animation_controller != 0 {
                match read_remote_u64(
                    process.0,
                    animation_controller,
                    "animation controller vtable",
                ) {
                    Ok(vtable) => {
                        animation_controller_rtti = read_msvc_rtti_name(
                            process.0,
                            vtable,
                            client_range,
                            "animation controller",
                        )
                        .unwrap_or_else(|error| format!("RTTI_ERROR:{error}"));
                    }

                    Err(error) => {
                        animation_controller_rtti =
                            format!("VTABLE_ERROR:{error}");
                    }
                }
            }

            let scene_node = match read_remote_u64(
                process.0,
                checked_add(
                    pawn,
                    game_scene_node_offset as u64,
                    "game scene node member",
                )?,
                "game scene node",
            ) {
                Ok(value) if value != 0 => value,
                _ => continue,
            };

            let position = match read_remote_f32x3(
                process.0,
                checked_add(
                    scene_node,
                    absolute_origin_offset as u64,
                    "absolute origin member",
                )?,
                "absolute origin",
            ) {
                Ok(value) => value,
                Err(_) => continue,
            };

            let ang_rotation = read_remote_f32x3(
                process.0,
                checked_add(
                    scene_node,
                    ang_rotation_offset as u64,
                    "scene node rotation",
                )?,
                "scene node rotation",
            )
            .unwrap_or([0.0; 3]);

            let ang_abs_rotation = read_remote_f32x3(
                process.0,
                checked_add(
                    scene_node,
                    ang_abs_rotation_offset as u64,
                    "scene node absolute rotation",
                )?,
                "scene node absolute rotation",
            )
            .unwrap_or([0.0; 3]);

            let ang_wrapped_local_rotation = read_remote_f32x3(
                process.0,
                checked_add(
                    scene_node,
                    ang_wrapped_local_rotation_offset as u64,
                    "scene node wrapped local rotation",
                )?,
                "scene node wrapped local rotation",
            )
            .unwrap_or([0.0; 3]);

            let abs_velocity = read_remote_f32x3(
                process.0,
                pawn + 0x400,
                "m_vecAbsVelocity",
            )
            .unwrap_or([0.0; 3]);

            let server_velocity = read_remote_f32x3(
                process.0,
                pawn + 0x40C,
                "m_vecServerVelocity",
            )
            .unwrap_or([0.0; 3]);

            let velocity = read_remote_f32x3(
                process.0,
                pawn + 0x438,
                "m_vecVelocity",
            )
            .unwrap_or([0.0; 3]);

            let last_velocity = read_remote_f32x3(
                process.0,
                pawn + 0x16B8,
                "m_vLastVelocity",
            )
            .unwrap_or([0.0; 3]);

            let movement_services = read_remote_u64(
                process.0,
                checked_add(
                    pawn,
                    0xEC0,
                    "movement services member",
                )?,
                "movement services",
            )
            .unwrap_or(0);

            let (
                pawn_turn_spring_speed,
                ag2_turn_speed,
                ag2_turn_speed_spring_speed,
                input_direction_commitment,
                last_wish_dir,
            ) = if movement_services != 0 {
                (
                    read_remote_f32(
                        process.0,
                        movement_services + 0x2DC,
                        "m_flPawnTurnSpringSpeed",
                    )
                    .unwrap_or(0.0),

                    read_remote_f32(
                        process.0,
                        movement_services + 0x2E0,
                        "m_flAG2TurnSpeed",
                    )
                    .unwrap_or(0.0),

                    read_remote_f32(
                        process.0,
                        movement_services + 0x2E4,
                        "m_flAG2TurnSpeedSpringSpeed",
                    )
                    .unwrap_or(0.0),

                    read_remote_f32(
                        process.0,
                        movement_services + 0x2E8,
                        "m_flInputDirectionCommitment",
                    )
                    .unwrap_or(0.0),

                    read_remote_f32x2(
                        process.0,
                        movement_services + 0x2F4,
                        "m_vLastWishDir",
                    )
                    .unwrap_or([0.0; 2]),
                )
            } else {
                (
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    [0.0; 2],
                )
            };

            let pred_slow_speed = read_remote_f32(
                process.0,
                pawn + 0x1808,
                "m_flPredSlowSpeed",
            )
            .unwrap_or(0.0);

            let slow_speed = read_remote_f32(
                process.0,
                pawn + 0x182C,
                "m_flSlowSpeed",
            )
            .unwrap_or(0.0);

            let loco_flags = read_remote_u32(
                process.0,
                pawn + 0x1844,
                "loco flags",
            )
            .unwrap_or(0);

            let loco_lean_triggered = (loco_flags & 0xFF) != 0;
            let loco_run_to_stop = ((loco_flags >> 8) & 0xFF) != 0;

            let eye_angles = read_remote_f32x3(
                process.0,
                pawn + 0x1108,
                "m_angEyeAngles",
            )
            .unwrap_or([0.0; 3]);

            let locked_eye_angles = read_remote_f32x3(
                process.0,
                pawn + 0x1430,
                "m_angLockedEyeAngles",
            )
            .unwrap_or([0.0; 3]);

            let graph_instance_ag2 = if animation_controller != 0 {
                read_remote_u64(
                    process.0,
                    animation_controller + 0x3C8,
                    "m_pGraphInstanceAG2",
                )
                .unwrap_or(0)
            } else {
                0
            };

            let mut graph_instance_rtti = String::from("null");

            if graph_instance_ag2 != 0 {
                match read_remote_u64(
                    process.0,
                    graph_instance_ag2,
                    "AG2 graph instance vtable",
                ) {
                    Ok(vtable) => {
                        graph_instance_rtti = read_msvc_rtti_name(
                            process.0,
                            vtable,
                            client_range,
                            "AG2 graph instance",
                        )
                        .unwrap_or_else(|error| format!("RTTI_ERROR:{error}"));
                    }

                    Err(error) => {
                        graph_instance_rtti =
                            format!("VTABLE_ERROR:{error}");
                    }
                }
            }

            let graph_definition_ag2 = if animation_controller != 0 {
                read_remote_u64(
                    process.0,
                    animation_controller + 0x2F0,
                    "m_hGraphDefinitionAG2",
                )
                .unwrap_or(0)
            } else {
                0
            };

            let primary_graph_id = if animation_controller != 0 {
                read_remote_u64(
                    process.0,
                    animation_controller + 0x388,
                    "m_primaryGraphId",
                )
                .unwrap_or(0)
            } else {
                0
            };

            

        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok(())
}

fn read_chunks(process: HANDLE, entity_system: u64) -> Result<Vec<u64>, String> {
    let address = checked_add(entity_system, ENTITY_LIST_OFFSET, "entity chunk array")?;
    let bytes = read_exact(
        process,
        address,
        ENTITY_CHUNK_COUNT * 8,
        "entity chunk array",
    )?;
    let mut chunks = Vec::with_capacity(ENTITY_CHUNK_COUNT);
    for index in 0..ENTITY_CHUNK_COUNT {
        let chunk = read_u64(&bytes, index * 8, "entity chunk pointer")?;
        if chunk != 0 {
            validate_user_address(chunk, "entity chunk pointer")?;
        }
        chunks.push(chunk);
    }
    if chunks.iter().all(|chunk| *chunk == 0) {
        return Err("Client entity system has no populated entity chunks".to_string());
    }
    Ok(chunks)
}

fn identity_from_chunk(
    chunk: &[u8],
    slot: usize,
    identity_size: usize,
) -> Result<EntityIdentity, String> {
    if slot >= ENTITIES_PER_CHUNK {
        return Err(format!("Entity slot {slot} is out of range"));
    }
    let offset = slot
        .checked_mul(identity_size)
        .ok_or_else(|| "Entity identity offset overflow".to_string())?;
    Ok(EntityIdentity {
        instance: read_u64(chunk, offset, "entity instance")?,
        handle: read_u32(
            chunk,
            offset + ENTITY_IDENTITY_HANDLE_OFFSET,
            "entity identity handle",
        )?,
    })
}

fn resolve_handle(
    process: HANDLE,
    chunks: &[u64],
    handle: u32,
    identity_size: usize,
) -> Result<u64, String> {
    let index = handle_index(handle)?;
    let chunk_index = validate_entity_index(index, chunks.len())?;
    let slot = index % ENTITIES_PER_CHUNK;
    let chunk = *chunks
        .get(chunk_index)
        .ok_or_else(|| format!("Handle index {index} exceeds entity chunks"))?;
    if chunk == 0 {
        return Err(format!(
            "Handle 0x{handle:08X} points to a null entity chunk"
        ));
    }
    let identity_address = checked_add(chunk, (slot * identity_size) as u64, "handle identity")?;
    let bytes = read_exact(process, identity_address, identity_size, "handle identity")?;
    let identity = identity_from_chunk(&bytes, 0, identity_size)?;
    validate_handle_match(handle, identity.handle)?;
    if identity.instance == 0 {
        return Err(format!("Handle 0x{handle:08X} resolves to a null instance"));
    }
    validate_readable_range(process, identity.instance, 8, "resolved entity instance")?;
    Ok(identity.instance)
}

fn handle_index(handle: u32) -> Result<usize, String> {
    if handle == 0 || handle == u32::MAX {
        return Err(format!("Invalid null entity handle 0x{handle:08X}"));
    }
    let index = (handle & ENTITY_INDEX_MASK) as usize;
    if index >= MAX_ENTITIES {
        return Err(format!(
            "Entity handle index {index} exceeds limit {MAX_ENTITIES}"
        ));
    }
    Ok(index)
}

fn validate_entity_index(index: usize, chunk_count: usize) -> Result<usize, String> {
    if index >= MAX_ENTITIES {
        return Err(format!("Entity index {index} exceeds limit {MAX_ENTITIES}"));
    }
    let chunk_index = index / ENTITIES_PER_CHUNK;
    if chunk_index >= chunk_count {
        return Err(format!(
            "Entity index {index} exceeds the available chunk table"
        ));
    }
    Ok(chunk_index)
}

fn identity_index(handle: u32) -> Option<usize> {
    if handle == 0 || handle == u32::MAX {
        None
    } else {
        Some((handle & ENTITY_INDEX_MASK) as usize)
    }
}

fn validate_identity_size(size: u32) -> Result<usize, String> {
    let size = size as usize;
    if size < ENTITY_IDENTITY_HANDLE_OFFSET + 4 || size > 0x200 || size % 8 != 0 {
        return Err(format!(
            "Implausible CEntityIdentity schema size: 0x{size:X}"
        ));
    }
    Ok(size)
}

fn validate_handle_match(requested: u32, actual: u32) -> Result<(), String> {
    if requested != actual {
        return Err(format!("Entity handle serial mismatch: requested 0x{requested:08X}, identity has 0x{actual:08X}"));
    }
    Ok(())
}

fn is_local_controller_flag(value: u8) -> bool {
    value == 1
}

fn validate_object_vtable(
    process: HANDLE,
    object: u64,
    required_rtti_fragment: Option<&str>,
    module: (u64, u64),
    label: &str,
) -> Result<(), String> {
    validate_readable_range(process, object, 8, label)?;
    let vtable = read_remote_u64(process, object, &format!("{label} vtable"))?;
    validate_module_address(vtable, module, &format!("{label} vtable"))?;
    if let Some(fragment) = required_rtti_fragment {
        let name = read_msvc_rtti_name(process, vtable, module, label)?;
        if !name.contains(fragment) {
            return Err(format!(
                "{label} RTTI {name:?} does not contain {fragment:?}"
            ));
        }
    }
    Ok(())
}

fn read_msvc_rtti_name(
    process: HANDLE,
    vtable: u64,
    module: (u64, u64),
    label: &str,
) -> Result<String, String> {
    let locator_pointer = vtable
        .checked_sub(8)
        .ok_or_else(|| format!("{label} vtable underflow"))?;
    let locator = read_remote_u64(process, locator_pointer, &format!("{label} RTTI locator"))?;
    validate_module_address(locator, module, &format!("{label} RTTI locator"))?;
    let header = read_exact(process, locator, 24, &format!("{label} RTTI header"))?;
    if read_u32(&header, 0, "RTTI signature")? != 1 {
        return Err(format!("{label} has unsupported RTTI signature"));
    }
    let self_rva = read_u32(&header, 20, "RTTI self RVA")? as u64;
    if module.0.checked_add(self_rva) != Some(locator) {
        return Err(format!("{label} RTTI self pointer is inconsistent"));
    }
    let type_rva = read_u32(&header, 12, "RTTI type RVA")? as u64;
    let type_descriptor = module
        .0
        .checked_add(type_rva)
        .ok_or_else(|| "RTTI type address overflow".to_string())?;
    validate_module_address(
        type_descriptor,
        module,
        &format!("{label} RTTI type descriptor"),
    )?;
    let name = read_exact(
        process,
        checked_add(type_descriptor, 16, "RTTI name")?,
        MAX_RTTI_NAME,
        "RTTI name",
    )?;
    let end = name
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| format!("{label} RTTI name is not terminated"))?;
    let value = std::str::from_utf8(&name[..end])
        .map_err(|_| format!("{label} RTTI name is not UTF-8"))?
        .to_string();
    if !value.starts_with(".?AV") {
        return Err(format!("{label} has implausible RTTI name {value:?}"));
    }
    Ok(value)
}

fn validate_module_address(address: u64, module: (u64, u64), label: &str) -> Result<(), String> {
    let end = module
        .0
        .checked_add(module.1)
        .ok_or_else(|| "Module range overflow".to_string())?;
    if address < module.0 || address >= end {
        return Err(format!("{label} 0x{address:X} is outside client.dll"));
    }
    Ok(())
}

fn open_process_read_only(pid: u32) -> Result<ReadOnlyProcess, String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        return Err(format!(
            "Could not open Deadlock PID {pid} for pawn discovery: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(ReadOnlyProcess(handle))
}

fn read_exact(process: HANDLE, address: u64, size: usize, label: &str) -> Result<Vec<u8>, String> {
    validate_readable_range(process, address, size, label)?;
    let address =
        usize::try_from(address).map_err(|_| format!("{label} address does not fit usize"))?;
    let bytes = camera::read_bytes(process, address, size)?;
    if bytes.len() != size {
        return Err(format!(
            "Incomplete {label} read: expected {size}, got {}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

fn read_remote_u64(process: HANDLE, address: u64, label: &str) -> Result<u64, String> {
    read_u64(&read_exact(process, address, 8, label)?, 0, label)
}

fn read_remote_u32(process: HANDLE, address: u64, label: &str) -> Result<u32, String> {
    read_u32(&read_exact(process, address, 4, label)?, 0, label)
}

fn read_remote_f32(
    process: HANDLE,
    address: u64,
    label: &str,
) -> Result<f32, String> {
    let bytes = read_exact(process, address, 4, label)?;

    Ok(f32::from_le_bytes(
        bytes.try_into()
            .map_err(|_| format!("Invalid f32 read for {label}"))?,
    ))
}

fn read_remote_f32x3(
    process: HANDLE,
    address: u64,
    label: &str,
) -> Result<[f32; 3], String> {
    let bytes = read_exact(process, address, 12, label)?;
    read_f32x3(&bytes, 0, label)
}

fn read_remote_f32x2(
    process: HANDLE,
    address: u64,
    label: &str,
) -> Result<[f32; 2], String> {
    Ok([
        read_remote_f32(process, address, label)?,
        read_remote_f32(
            process,
            checked_add(address, 4, label)?,
            label,
        )?,
    ])
}

fn read_f32x3(bytes: &[u8], offset: usize, label: &str) -> Result<[f32; 3], String> {
    let mut result = [0.0; 3];
    for (index, value) in result.iter_mut().enumerate() {
        let start = offset
            .checked_add(index * 4)
            .ok_or_else(|| format!("{label} offset overflow"))?;
        *value = f32::from_le_bytes(
            bytes
                .get(start..start + 4)
                .ok_or_else(|| format!("{label} is outside its snapshot"))?
                .try_into()
                .map_err(|_| format!("Invalid {label}"))?,
        );
    }
    Ok(result)
}

fn read_f32(bytes: &[u8], offset: usize, label: &str) -> Result<f32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| format!("{label} is outside the telemetry snapshot"))?;
    Ok(f32::from_le_bytes(
        value.try_into().expect("four-byte slice"),
    ))
}

fn validate_readable_range(
    process: HANDLE,
    address: u64,
    size: usize,
    label: &str,
) -> Result<(), String> {
    if size == 0 {
        return Err(format!("{label} range is empty"));
    }
    validate_user_address(address, label)?;
    let end = address
        .checked_add(size as u64)
        .ok_or_else(|| format!("{label} range overflow"))?;
    validate_user_address(end - 1, label)?;
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
    let queried = unsafe {
        VirtualQueryEx(
            process,
            address as usize as *const _,
            &mut info,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if queried != size_of::<MEMORY_BASIC_INFORMATION>() {
        return Err(format!(
            "VirtualQueryEx failed for {label}: {}",
            std::io::Error::last_os_error()
        ));
    }
    if info.State != MEM_COMMIT
        || info.Protect == 0
        || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
    {
        return Err(format!("{label} is not readable committed memory"));
    }
    let region_base = info.BaseAddress as usize as u64;
    let region_end = region_base
        .checked_add(info.RegionSize as u64)
        .ok_or_else(|| "Memory region overflow".to_string())?;
    if address < region_base || end > region_end {
        return Err(format!("{label} crosses a memory-region boundary"));
    }
    Ok(())
}

fn validate_user_address(address: u64, label: &str) -> Result<(), String> {
    if !(MIN_USER_ADDRESS..=MAX_USER_ADDRESS).contains(&address) {
        return Err(format!(
            "{label} is not a canonical user address: 0x{address:X}"
        ));
    }
    Ok(())
}

fn checked_add(address: u64, offset: u64, label: &str) -> Result<u64, String> {
    address
        .checked_add(offset)
        .ok_or_else(|| format!("{label} address overflow"))
}

fn read_u32(bytes: &[u8], offset: usize, label: &str) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{label} is outside its buffer"))?;
    Ok(u32::from_le_bytes(
        value.try_into().map_err(|_| format!("Invalid {label}"))?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize, label: &str) -> Result<u64, String> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| format!("{label} is outside its buffer"))?;
    Ok(u64::from_le_bytes(
        value.try_into().map_err(|_| format!("Invalid {label}"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_decoding_rejects_null_and_out_of_range_values() {
        assert!(handle_index(0).is_err());
        assert!(handle_index(u32::MAX).is_err());
        assert_eq!(handle_index(0x0000_1234), Ok(0x1234));
        assert!(validate_entity_index(MAX_ENTITIES, ENTITY_CHUNK_COUNT).is_err());
        assert!(validate_entity_index(ENTITIES_PER_CHUNK, 1).is_err());
    }

    #[test]
    fn handle_serial_mismatch_is_rejected() {
        assert!(validate_handle_match(0x0001_0012, 0x0002_0012).is_err());
        assert!(validate_handle_match(0x0001_0012, 0x0001_0012).is_ok());
    }

    #[test]
    fn controller_local_flag_requires_canonical_true() {
        assert!(is_local_controller_flag(1));
        assert!(!is_local_controller_flag(0));
        assert!(!is_local_controller_flag(2));
    }

    #[test]
    fn identity_slot_limit_is_enforced() {
        let identity_size = 0x70;
        let chunk = vec![0_u8; identity_size * ENTITIES_PER_CHUNK];
        assert!(identity_from_chunk(&chunk, ENTITIES_PER_CHUNK, identity_size).is_err());
    }

    #[test]
    fn identity_size_is_bounded_and_aligned() {
        assert_eq!(validate_identity_size(0x70), Ok(0x70));
        assert!(validate_identity_size(0x13).is_err());
        assert!(validate_identity_size(0x208).is_err());
    }

    #[test]
    fn telemetry_field_span_covers_every_field_once() {
        assert_eq!(
            field_span(&[(0x330, 8), (0x400, 12), (0x1038, 12)]),
            Ok((0x330, 0xD14))
        );
        assert!(field_span(&[]).is_err());
    }

    #[test]
    fn telemetry_snapshot_match_is_bit_exact() {
        let snapshot = TelemetrySnapshot {
            scene_node: 1,
            position: [1.0, 2.0, 3.0],
            absolute_origin: [1.0, 2.0, 3.0],
            wrapped_local_origin: [1.0, 2.0, 3.0],
            velocity: [4.0, 5.0, 6.0],
            old_origin: [0.0, 1.0, 2.0],
            view_angles: [-10.0, 90.0, 0.0],
            client_camera_angles: [-10.0, 90.0, 0.0],
            prediction_error: [0.0; 3],
            prediction_error_time: 0.0,
            simulation_tick: 42,
            simulation_time: 1.5,
        };
        assert!(snapshots_match(&snapshot, &snapshot));

        let mut changed = snapshot;
        changed.view_angles[1] = 91.0;
        assert!(!snapshots_match(&snapshot, &changed));
    }
}
