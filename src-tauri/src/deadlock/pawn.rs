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
}
