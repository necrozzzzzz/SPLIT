use std::{
    collections::HashSet,
    mem::{size_of, zeroed},
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        Memory::{VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS},
        Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
    },
};

use super::camera;

// Current Source 2 ABI candidate. This is accepted only after the vector and
// every scope/name reachable through it pass the validations below.
const TYPE_SCOPES_OFFSET: u64 = 0x190;
const TYPE_SCOPE_NAME_OFFSET: u64 = 0x8;
const TYPE_SCOPE_NAME_CAPACITY: usize = 256;
const UTL_VECTOR_SIZE: usize = 16;
const POINTER_SIZE: usize = 8;
const MAX_TYPE_SCOPES: usize = 128;
const MAX_SCOPE_LOGS: usize = 32;
const CLASS_BINDINGS_OFFSET: u64 = 0x560;
const UTL_TS_HASH_BUCKETS_OFFSET: usize = 0x60;
const UTL_TS_HASH_BUCKET_COUNT: usize = 256;
const UTL_TS_HASH_BUCKET_SIZE: usize = 0x18;
const UTL_TS_HASH_SIZE: usize = 0x1870;
const HASH_BUCKET_FIRST_UNCOMMITTED_OFFSET: usize = 0x10;
const MEMORY_POOL_FREE_HEAD_OFFSET: usize = 0x20;
const MAX_CLASS_BINDINGS: usize = 32_768;
const MAX_SCHEMA_NAME: usize = 256;
const FIXED_HASH_NODE_SIZE: usize = 0x18;
const ALLOCATED_HASH_NODE_SIZE: usize = 0x30;
const CLASS_BINDING_READ_SIZE: usize = 0x38;
const CLASS_FIELD_SIZE: usize = 0x20;
const MAX_CLASS_FIELDS: usize = 1_024;
const MAX_CLASS_SIZE: i32 = 1024 * 1024;
const MIN_USER_ADDRESS: u64 = 0x1_0000;
const MAX_USER_ADDRESS: u64 = 0x0000_7FFF_FFFF_FFFF;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopeEntry {
    address: u64,
    name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HashNode {
    next: u64,
    data: u64,
}

#[derive(Debug)]
struct ClassHashLayout {
    blocks_allocated: usize,
    peak_allocated: usize,
    free_head: u64,
    bucket_heads: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClassLayout {
    size: i32,
    field_count: usize,
    fields: u64,
}

struct ReadOnlyProcess(HANDLE);

impl Drop for ReadOnlyProcess {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub(crate) struct SchemaResolver {
    _process: ReadOnlyProcess,
    _schema_system: u64,
    scopes: Vec<ScopeEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RouteSchema {
    pub(crate) local_flag_offset: u32,
    pub(crate) pawn_handle_offset: u32,
    pub(crate) identity_size: u32,
    pub(crate) game_scene_node_offset: u32,
    pub(crate) local_origin_offset: u32,
    pub(crate) absolute_origin_offset: u32,
    pub(crate) wrapped_local_origin_offset: u32,

    pub(crate) scene_node_ang_rotation_offset: u32,
    pub(crate) scene_node_ang_abs_rotation_offset: u32,
    pub(crate) scene_node_ang_wrapped_local_rotation_offset: u32,

    pub(crate) absolute_velocity_offset: u32,
    pub(crate) simulation_tick_offset: u32,
    pub(crate) simulation_time_offset: u32,
    pub(crate) old_origin_offset: u32,
    pub(crate) view_angles_offset: u32,
    pub(crate) client_camera_angles_offset: u32,
    pub(crate) prediction_error_offset: u32,
    pub(crate) prediction_error_time_offset: u32,
}

impl SchemaResolver {
    pub(crate) fn from_schema_system(pid: u32, schema_system: u64) -> Result<Self, String> {
        let process = open_process_read_only(pid)?;
        validate_readable_range(process.0, schema_system, 1, "SchemaSystem instance")?;

        let vector_address = checked_add(
            schema_system,
            TYPE_SCOPES_OFFSET,
            "SchemaSystem type-scopes vector",
        )?;
        let vector = read_exact(
            process.0,
            vector_address,
            UTL_VECTOR_SIZE,
            "SchemaSystem type-scopes vector",
        )?;

        // Current UtlVector<T> ABI:
        // +0x00 signed count, +0x04 reserved, +0x08 T* data.
        let count = read_i32(&vector, 0, "type-scopes count")?;
        let data = read_u64(&vector, 8, "type-scopes data pointer")?;
        let count = validate_vector_header(count, data)?;
        let readable_bytes = readable_bytes_from(process.0, data, "type-scopes data allocation")?;
        let readable_slots = readable_bytes / POINTER_SIZE;
        validate_vector_capacity(count, readable_slots)?;
        let pointer_bytes = count
            .checked_mul(POINTER_SIZE)
            .ok_or_else(|| "Type-scope pointer-array size overflow".to_string())?;
        let pointers = read_exact(process.0, data, pointer_bytes, "type-scopes pointer array")?;

        let mut scopes = Vec::with_capacity(count);
        for index in 0..count {
            let pointer_offset = index
                .checked_mul(POINTER_SIZE)
                .ok_or_else(|| "Type-scope pointer offset overflow".to_string())?;
            let address = read_u64(&pointers, pointer_offset, "type-scope pointer")?;
            if address == 0 {
                return Err(format!("Type-scope pointer {index} is null"));
            }

            let name_address =
                checked_add(address, TYPE_SCOPE_NAME_OFFSET, "type-scope name address")?;
            let name_bytes = read_exact(
                process.0,
                name_address,
                TYPE_SCOPE_NAME_CAPACITY,
                "type-scope name",
            )?;
            let name = parse_scope_name(&name_bytes)?;
            scopes.push(ScopeEntry { address, name });
        }

        if !scopes
            .iter()
            .any(|scope| is_supported_client_scope_name(&scope.name))
        {
            return Err(
                "Validated +0x190 vector contains no exact client/client.dll scope".to_string(),
            );
        }

        Ok(Self {
            _process: process,
            _schema_system: schema_system,
            scopes,
        })
    }

    pub(crate) fn find_scope(&self, scope_name: &str) -> Result<Option<u64>, String> {
        if scope_name.is_empty() || scope_name.len() >= TYPE_SCOPE_NAME_CAPACITY {
            return Err("Scope lookup name length is invalid".to_string());
        }

        Ok(self
            .scopes
            .iter()
            .find(|scope| scope_name_matches(&scope.name, scope_name))
            .map(|scope| scope.address))
    }

    pub(crate) fn find_class(&self, scope: u64, class_name: &str) -> Result<Option<u64>, String> {
        validate_schema_identifier(class_name, "Class lookup name")?;
        validate_readable_range(self._process.0, scope, 1, "schema type scope")?;

        let hash_address = checked_add(scope, CLASS_BINDINGS_OFFSET, "class-bindings hash")?;
        let layout = self.read_class_hash_layout(hash_address)?;
        let mut seen_bindings = HashSet::new();

        let bucket_bindings =
            traverse_hash_nodes(&layout.bucket_heads, layout.blocks_allocated, |node| {
                self.read_fixed_hash_node(node)
            })?;
        if let Some(binding) =
            self.match_class_binding(&bucket_bindings, &mut seen_bindings, class_name)?
        {
            return Ok(Some(binding));
        }

        if layout.free_head != 0 && layout.peak_allocated > 0 {
            let allocated_bindings =
                traverse_hash_nodes(&[layout.free_head], layout.peak_allocated, |node| {
                    self.read_allocated_hash_node(node)
                })?;
            if let Some(binding) =
                self.match_class_binding(&allocated_bindings, &mut seen_bindings, class_name)?
            {
                return Ok(Some(binding));
            }
        }

        Ok(None)
    }

    pub(crate) fn find_field_offset(
        &self,
        scope: u64,
        class_name: &str,
        field_name: &str,
    ) -> Result<Option<u32>, String> {
        validate_schema_identifier(field_name, "Field lookup name")?;
        let Some(class) = self.find_class(scope, class_name)? else {
            return Ok(None);
        };
        let layout = self.read_class_layout(class)?;
        self.find_field_in_layout(layout, field_name)
    }

    pub(crate) fn find_class_size(
        &self,
        scope: u64,
        class_name: &str,
    ) -> Result<Option<u32>, String> {
        let Some(class) = self.find_class(scope, class_name)? else {
            return Ok(None);
        };
        let size = self.read_class_layout(class)?.size;
        u32::try_from(size)
            .map(Some)
            .map_err(|_| format!("Schema class {class_name} has an invalid size: {size}"))
    }

    fn read_class_hash_layout(&self, hash_address: u64) -> Result<ClassHashLayout, String> {
        let hash = read_exact(
            self._process.0,
            hash_address,
            UTL_TS_HASH_SIZE,
            "class-bindings UtlTsHash",
        )?;

        let block_size = read_i32(&hash, 0x00, "class hash block_size")?;
        let blocks_per_blob = read_i32(&hash, 0x04, "class hash blocks_per_blob")?;
        let grow_mode = read_u32(&hash, 0x08, "class hash grow_mode")?;
        let blocks_allocated = read_i32(&hash, 0x0C, "class hash blocks_allocated")?;
        let peak_allocated = read_i32(&hash, 0x10, "class hash peak_allocated")?;
        let alignment = read_u16(&hash, 0x14, "class hash alignment")?;
        let blob_count = read_u16(&hash, 0x16, "class hash blob_count")?;
        let free_head = read_u64(&hash, MEMORY_POOL_FREE_HEAD_OFFSET, "class hash free head")?;

        let (blocks_allocated, peak_allocated) = validate_hash_pool_metadata(
            block_size,
            blocks_per_blob,
            grow_mode,
            blocks_allocated,
            peak_allocated,
            alignment,
            blob_count,
        )?;

        if free_head != 0 {
            validate_user_address(free_head, "class hash free head")?;
        }

        let mut bucket_heads = Vec::new();
        for index in 0..UTL_TS_HASH_BUCKET_COUNT {
            let bucket_offset = UTL_TS_HASH_BUCKETS_OFFSET
                .checked_add(index * UTL_TS_HASH_BUCKET_SIZE)
                .and_then(|offset| offset.checked_add(HASH_BUCKET_FIRST_UNCOMMITTED_OFFSET))
                .ok_or_else(|| "Class hash bucket offset overflow".to_string())?;
            let head = read_u64(&hash, bucket_offset, "class hash bucket head")?;
            if head != 0 {
                validate_user_address(head, "class hash bucket head")?;
                bucket_heads.push(head);
            }
        }
        if bucket_heads.is_empty() {
            return Err("Validated class-bindings hash has no populated buckets".to_string());
        }

        Ok(ClassHashLayout {
            blocks_allocated,
            peak_allocated,
            free_head,
            bucket_heads,
        })
    }

    fn read_fixed_hash_node(&self, address: u64) -> Result<HashNode, String> {
        let node = read_exact(
            self._process.0,
            address,
            FIXED_HASH_NODE_SIZE,
            "class hash fixed node",
        )?;
        Ok(HashNode {
            next: read_u64(&node, 0x08, "class hash fixed-node next")?,
            data: read_u64(&node, 0x10, "class hash fixed-node data")?,
        })
    }

    fn read_allocated_hash_node(&self, address: u64) -> Result<HashNode, String> {
        let node = read_exact(
            self._process.0,
            address,
            ALLOCATED_HASH_NODE_SIZE,
            "class hash allocated node",
        )?;
        Ok(HashNode {
            next: read_u64(&node, 0x00, "class hash allocated-node next")?,
            data: read_u64(&node, 0x10, "class hash allocated-node data")?,
        })
    }

    fn match_class_binding(
        &self,
        bindings: &[u64],
        seen_bindings: &mut HashSet<u64>,
        class_name: &str,
    ) -> Result<Option<u64>, String> {
        for &binding in bindings {
            if binding == 0 || !seen_bindings.insert(binding) {
                continue;
            }
            validate_user_address(binding, "schema class binding")?;
            let name_pointer_address = checked_add(binding, 0x08, "class name field")?;
            let name_pointer = read_remote_u64(
                self._process.0,
                name_pointer_address,
                "schema class name pointer",
            )?;
            let name = read_bounded_c_string(
                self._process.0,
                name_pointer,
                MAX_SCHEMA_NAME,
                "schema class name",
            )?;
            if class_name_matches(&name, class_name) {
                return Ok(Some(binding));
            }
        }
        Ok(None)
    }

    fn read_class_layout(&self, class: u64) -> Result<ClassLayout, String> {
        let binding = read_exact(
            self._process.0,
            class,
            CLASS_BINDING_READ_SIZE,
            "SchemaClassInfoData",
        )?;
        let size = read_i32(&binding, 0x20, "schema class size")?;
        let field_count = read_i16(&binding, 0x24, "schema class field_count")?;
        let fields = read_u64(&binding, 0x30, "schema class fields pointer")?;
        let field_count = validate_class_fields(field_count, fields)?;
        if size <= 0 || size > MAX_CLASS_SIZE {
            return Err(format!("Invalid schema class size: {size}"));
        }

        let fields_size = field_count
            .checked_mul(CLASS_FIELD_SIZE)
            .ok_or_else(|| "Schema field-array size overflow".to_string())?;
        validate_readable_range(
            self._process.0,
            fields,
            fields_size,
            "SchemaClassFieldData array",
        )?;

        Ok(ClassLayout {
            size,
            field_count,
            fields,
        })
    }

    fn find_field_in_layout(
        &self,
        layout: ClassLayout,
        field_name: &str,
    ) -> Result<Option<u32>, String> {
        let fields_size = layout
            .field_count
            .checked_mul(CLASS_FIELD_SIZE)
            .ok_or_else(|| "Schema field-array size overflow".to_string())?;
        let fields = read_exact(
            self._process.0,
            layout.fields,
            fields_size,
            "SchemaClassFieldData array",
        )?;

        for index in 0..layout.field_count {
            let field_offset = index
                .checked_mul(CLASS_FIELD_SIZE)
                .ok_or_else(|| "Schema field offset overflow".to_string())?;
            let name_pointer = read_u64(&fields, field_offset, "schema field name pointer")?;
            let name = read_bounded_c_string(
                self._process.0,
                name_pointer,
                MAX_SCHEMA_NAME,
                "schema field name",
            )?;
            if field_name_matches(&name, field_name) {
                let offset = read_i32(&fields, field_offset + 0x10, "schema field runtime offset")?;
                return validate_field_offset(offset, layout.size).map(Some);
            }
        }

        Ok(None)
    }

    pub(crate) fn debug_list_fields(
        &self,
        scope: u64,
        class_name: &str,
    ) -> Result<Vec<(String, u32)>, String> {
        let class = self
            .find_class(scope, class_name)?
            .ok_or_else(|| format!("Schema class {class_name} was not found"))?;

        let layout = self.read_class_layout(class)?;

        let fields_size = layout
            .field_count
            .checked_mul(CLASS_FIELD_SIZE)
            .ok_or_else(|| "Schema field-array size overflow".to_string())?;

        let fields = read_exact(
            self._process.0,
            layout.fields,
            fields_size,
            "SchemaClassFieldData array",
        )?;

        let mut result = Vec::new();

        for index in 0..layout.field_count {
            let field_offset = index * CLASS_FIELD_SIZE;

            let name_pointer =
                read_u64(&fields, field_offset, "schema field name pointer")?;

            let name = read_bounded_c_string(
                self._process.0,
                name_pointer,
                MAX_SCHEMA_NAME,
                "schema field name",
            )?;

            let offset =
                read_i32(&fields, field_offset + 0x10, "schema field runtime offset")?;

            if let Ok(offset) = validate_field_offset(offset, layout.size) {
                result.push((name, offset));
            }
        }

        Ok(result)
    }

    fn scopes(&self) -> &[ScopeEntry] {
        &self.scopes
    }
}

pub(crate) fn resolve_route_schema(pid: u32) -> Result<RouteSchema, String> {
    let schema_system =
        super::interfaces::find_interface(pid, "schemasystem.dll", "SchemaSystem_001")?
            .ok_or_else(|| "Exact interface SchemaSystem_001 was not found".to_string())?;
    let resolver = SchemaResolver::from_schema_system(pid, schema_system)?;
    let client = resolver
        .find_scope("client")?
        .or(resolver.find_scope("client.dll")?)
        .ok_or_else(|| "Exact client/client.dll scope was not found".to_string())?;

    println!("[SPLIT][Schema][Class] CBodyComponentBaseAnimGraph");

    match resolver.debug_list_fields(client, "CBodyComponentBaseAnimGraph") {
        Ok(fields) => {
            println!(
                "[SPLIT][Schema][ClassFields] CBodyComponentBaseAnimGraph count={}",
                fields.len()
            );

            for (name, offset) in fields {
                println!(
                    "[SPLIT][Schema][Field] CBodyComponentBaseAnimGraph::{name} = 0x{offset:X}"
                );
            }
        }

        Err(error) => {
            println!(
                "[SPLIT][Schema][ClassError] CBodyComponentBaseAnimGraph: {error}"
            );
        }
    }
        
    let required = |class_name: &str, field_name: &str| -> Result<u32, String> {
        resolver
            .find_field_offset(client, class_name, field_name)?
            .ok_or_else(|| format!("Exact schema field {class_name}::{field_name} was not found"))
    };

    Ok(RouteSchema {
        local_flag_offset: required("CBasePlayerController", "m_bIsLocalPlayerController")?,
        pawn_handle_offset: required("CBasePlayerController", "m_hPawn")?,
        identity_size: resolver
            .find_class_size(client, "CEntityIdentity")?
            .ok_or_else(|| "Exact schema class CEntityIdentity was not found".to_string())?,
        game_scene_node_offset: required("C_BaseEntity", "m_pGameSceneNode")?,
        local_origin_offset: required("CGameSceneNode", "m_vecOrigin")?,
        absolute_origin_offset: required("CGameSceneNode", "m_vecAbsOrigin")?,
        wrapped_local_origin_offset: required("CGameSceneNode", "m_vecWrappedLocalOrigin")?,

        scene_node_ang_rotation_offset: required(
            "CGameSceneNode",
            "m_angRotation",
        )?,

        scene_node_ang_abs_rotation_offset: required(
            "CGameSceneNode",
            "m_angAbsRotation",
        )?,

        scene_node_ang_wrapped_local_rotation_offset: required(
            "CGameSceneNode",
            "m_angWrappedLocalRotation",
        )?,

        absolute_velocity_offset: required("C_BaseEntity", "m_vecAbsVelocity")?,
        simulation_tick_offset: required("C_BaseEntity", "m_nSimulationTick")?,
        simulation_time_offset: required("C_BaseEntity", "m_flSimulationTime")?,
        old_origin_offset: required("C_BasePlayerPawn", "m_vOldOrigin")?,
        view_angles_offset: required("C_BasePlayerPawn", "v_angle")?,
        client_camera_angles_offset: required("C_CitadelPlayerPawn", "m_angClientCamera")?,
        prediction_error_offset: required("C_BasePlayerPawn", "m_vecPredictionError")?,
        prediction_error_time_offset: required("C_BasePlayerPawn", "m_flPredictionErrorTime")?,
    })
}

pub(crate) fn initialize_client_runtime(pid: u32, schema_system: u64) -> Result<(), String> {
    let resolver = SchemaResolver::from_schema_system(pid, schema_system)?;

    let client = resolver
        .find_scope("client")?
        .or(resolver.find_scope("client.dll")?)
        .ok_or_else(|| "Exact client/client.dll scope was not found".to_string())?;

    const CLASS_NAME: &str = "C_CitadelPlayerPawn";
    const FIELD_NAME: &str = "m_nMapDistrictLocation";
    let class = resolver
        .find_class(client, CLASS_NAME)?
        .ok_or_else(|| format!("Exact schema class {CLASS_NAME} was not found"))?;

    let class_layout = resolver.read_class_layout(class)?;
    let offset = resolver
        .find_field_offset(client, CLASS_NAME, FIELD_NAME)?
        .ok_or_else(|| format!("Exact schema field {CLASS_NAME}::{FIELD_NAME} was not found"))?;
    #[cfg(debug_assertions)]
    super::district::debug_schema_offset(offset);

    const CONTROLLER_CLASS_NAME: &str = "CBasePlayerController";
    let mut controller_offsets = [0_u32; 2];
    for (index, controller_field) in ["m_bIsLocalPlayerController", "m_hPawn"]
        .into_iter()
        .enumerate()
    {
        let controller_offset = resolver
            .find_field_offset(client, CONTROLLER_CLASS_NAME, controller_field)?
            .ok_or_else(|| {
                format!(
                    "Exact schema field {CONTROLLER_CLASS_NAME}::{controller_field} was not found"
                )
            })?;
        controller_offsets[index] = controller_offset;
    }

    let identity_size = resolver
        .find_class_size(client, "CEntityIdentity")?
        .ok_or_else(|| "Exact schema class CEntityIdentity was not found".to_string())?;
    let initial_pawn = match super::pawn::log_local_pawn(
        pid,
        controller_offsets[0],
        controller_offsets[1],
        identity_size,
    ) {
        Ok(resolution) => Some(resolution),
        Err(error) => {
            eprintln!("[SPLIT][Pawn] local pawn initially unavailable: {error}");
            None
        }
    };
    super::district::start(
        pid,
        controller_offsets[0],
        controller_offsets[1],
        identity_size,
        offset,
        initial_pawn,
    )?;

    for (index, scope) in resolver.scopes().iter().take(MAX_SCOPE_LOGS).enumerate() {
        println!("[SPLIT][Schema] scope[{index}] name=\"{}\"", scope.name);
    }
    if resolver.scopes().len() > MAX_SCOPE_LOGS {
        println!(
            "[SPLIT][Schema] scope log truncated: {} of {} shown",
            MAX_SCOPE_LOGS,
            resolver.scopes().len()
        );
    }
    println!("[SPLIT][Schema] client scope = 0x{client:016X}");
    println!("[SPLIT][Schema] class {CLASS_NAME} = 0x{class:016X}");
    println!(
        "[SPLIT][Schema] class {CLASS_NAME} field_count={}",
        class_layout.field_count
    );
    println!("[SPLIT][Schema] {CLASS_NAME}::{FIELD_NAME} = 0x{offset:X}");
    for (controller_field, controller_offset) in [
        ("m_bIsLocalPlayerController", controller_offsets[0]),
        ("m_hPawn", controller_offsets[1]),
    ] {
        println!(
            "[SPLIT][Schema] {CONTROLLER_CLASS_NAME}::{controller_field} = 0x{controller_offset:X}"
        );
    }

    Ok(())
}

fn open_process_read_only(pid: u32) -> Result<ReadOnlyProcess, String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if process.is_null() {
        return Err(format!(
            "Could not open Deadlock PID {pid} for schema discovery: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(ReadOnlyProcess(process))
}

fn validate_hash_pool_metadata(
    block_size: i32,
    blocks_per_blob: i32,
    grow_mode: u32,
    blocks_allocated: i32,
    peak_allocated: i32,
    alignment: u16,
    blob_count: u16,
) -> Result<(usize, usize), String> {
    if block_size < FIXED_HASH_NODE_SIZE as i32 || block_size > 0x400 || block_size % 8 != 0 {
        return Err(format!("Invalid class hash block_size: {block_size}"));
    }
    if blocks_per_blob <= 0 || blocks_per_blob > MAX_CLASS_BINDINGS as i32 {
        return Err(format!(
            "Invalid class hash blocks_per_blob: {blocks_per_blob}"
        ));
    }
    if grow_mode > 2 {
        return Err(format!("Invalid class hash grow_mode: {grow_mode}"));
    }
    if blocks_allocated <= 0 || blocks_allocated > MAX_CLASS_BINDINGS as i32 {
        return Err(format!(
            "Invalid class hash blocks_allocated: {blocks_allocated}"
        ));
    }
    if peak_allocated < 0 || peak_allocated > MAX_CLASS_BINDINGS as i32 {
        return Err(format!(
            "Invalid class hash peak_allocated: {peak_allocated}"
        ));
    }
    if alignment == 0 || alignment > 0x100 || !alignment.is_power_of_two() {
        return Err(format!("Invalid class hash alignment: {alignment}"));
    }
    if blob_count as usize > MAX_CLASS_BINDINGS {
        return Err(format!("Invalid class hash blob_count: {blob_count}"));
    }
    Ok((blocks_allocated as usize, peak_allocated as usize))
}

fn traverse_hash_nodes<F>(
    heads: &[u64],
    max_nodes: usize,
    mut read_node: F,
) -> Result<Vec<u64>, String>
where
    F: FnMut(u64) -> Result<HashNode, String>,
{
    if max_nodes == 0 || max_nodes > MAX_CLASS_BINDINGS {
        return Err(format!("Invalid class hash traversal limit: {max_nodes}"));
    }

    let mut visited = HashSet::new();
    let mut data = Vec::new();
    for &head in heads {
        let mut node_address = head;
        while node_address != 0 {
            validate_user_address(node_address, "class hash node")?;
            if !visited.insert(node_address) {
                return Err(format!("Class hash cycle detected at 0x{node_address:X}"));
            }
            if visited.len() > max_nodes {
                return Err(format!("Class hash traversal exceeded {max_nodes} nodes"));
            }

            let node = read_node(node_address)?;
            if node.data != 0 {
                validate_user_address(node.data, "class hash binding")?;
                data.push(node.data);
            }
            node_address = node.next;
        }
    }
    Ok(data)
}

fn validate_class_fields(field_count: i16, fields: u64) -> Result<usize, String> {
    if field_count <= 0 {
        return Err(format!("Invalid schema field_count: {field_count}"));
    }
    let field_count = field_count as usize;
    if field_count > MAX_CLASS_FIELDS {
        return Err(format!(
            "Schema field_count {field_count} exceeds limit {MAX_CLASS_FIELDS}"
        ));
    }
    if fields == 0 {
        return Err("Schema fields pointer is null".to_string());
    }
    validate_user_address(fields, "schema fields pointer")?;
    Ok(field_count)
}

fn validate_field_offset(offset: i32, class_size: i32) -> Result<u32, String> {
    if offset < 0 {
        return Err(format!("Schema field offset is negative: {offset}"));
    }
    if class_size <= 0 || class_size > MAX_CLASS_SIZE {
        return Err(format!("Invalid schema class size: {class_size}"));
    }
    if offset >= class_size {
        return Err(format!(
            "Schema field offset 0x{offset:X} is outside class size 0x{class_size:X}"
        ));
    }
    Ok(offset as u32)
}

fn validate_schema_identifier(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() >= MAX_SCHEMA_NAME {
        return Err(format!("{label} length is invalid"));
    }
    if !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(format!("{label} contains invalid characters"));
    }
    Ok(())
}

fn class_name_matches(actual: &str, requested: &str) -> bool {
    actual == requested
}

fn field_name_matches(actual: &str, requested: &str) -> bool {
    actual == requested
}

fn read_remote_u64(process: HANDLE, address: u64, label: &str) -> Result<u64, String> {
    let bytes = read_exact(process, address, 8, label)?;
    read_u64(&bytes, 0, label)
}

fn read_bounded_c_string(
    process: HANDLE,
    address: u64,
    max_length: usize,
    label: &str,
) -> Result<String, String> {
    if address == 0 {
        return Err(format!("{label} pointer is null"));
    }
    let readable = readable_bytes_from(process, address, label)?;
    let read_size = readable.min(max_length);
    if read_size == 0 {
        return Err(format!("{label} has no readable bytes"));
    }
    let bytes = read_exact(process, address, read_size, label)?;
    let terminator = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| format!("{label} is not terminated within {max_length} bytes"))?;
    if terminator == 0 {
        return Err(format!("{label} is empty"));
    }
    let value = &bytes[..terminator];
    if !value.iter().all(|byte| byte.is_ascii_graphic()) {
        return Err(format!("{label} contains non-printable ASCII"));
    }
    String::from_utf8(value.to_vec()).map_err(|_| format!("{label} is not valid UTF-8"))
}

#[cfg(test)]
fn validate_vector_metadata(count: i32, data: u64, readable_slots: usize) -> Result<usize, String> {
    let count = validate_vector_header(count, data)?;
    validate_vector_capacity(count, readable_slots)?;
    Ok(count)
}

fn validate_vector_header(count: i32, data: u64) -> Result<usize, String> {
    if count <= 0 {
        return Err(format!("Invalid type-scope count: {count}"));
    }
    let count = count as usize;
    if count > MAX_TYPE_SCOPES {
        return Err(format!(
            "Type-scope count {count} exceeds limit {MAX_TYPE_SCOPES}"
        ));
    }
    if data == 0 {
        return Err("Type-scope data pointer is null".to_string());
    }
    validate_user_address(data, "type-scope data pointer")?;
    Ok(count)
}

fn validate_vector_capacity(count: usize, readable_slots: usize) -> Result<(), String> {
    if readable_slots < count {
        return Err(format!(
            "Type-scope allocation exposes {readable_slots} readable slots for count {count}"
        ));
    }
    Ok(())
}

fn parse_scope_name(bytes: &[u8]) -> Result<String, String> {
    let terminator = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| "Type-scope name is not terminated within 256 bytes".to_string())?;
    if terminator == 0 {
        return Err("Type-scope name is empty".to_string());
    }

    let name_bytes = &bytes[..terminator];
    if !name_bytes
        .iter()
        .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
    {
        return Err("Type-scope name contains non-printable ASCII".to_string());
    }
    String::from_utf8(name_bytes.to_vec())
        .map_err(|_| "Type-scope name is not valid UTF-8".to_string())
}

fn scope_name_matches(actual: &str, requested: &str) -> bool {
    actual.eq_ignore_ascii_case(requested)
}

fn is_supported_client_scope_name(name: &str) -> bool {
    scope_name_matches(name, "client") || scope_name_matches(name, "client.dll")
}

fn read_exact(process: HANDLE, address: u64, size: usize, label: &str) -> Result<Vec<u8>, String> {
    validate_readable_range(process, address, size, label)?;
    let address =
        usize::try_from(address).map_err(|_| format!("{label} address does not fit usize"))?;
    let bytes = camera::read_bytes(process, address, size)?;
    if bytes.len() != size {
        return Err(format!(
            "Incomplete {label} read: expected {size} bytes, got {}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

fn readable_bytes_from(process: HANDLE, address: u64, label: &str) -> Result<usize, String> {
    validate_user_address(address, label)?;
    let info = query_memory(process, address, label)?;
    validate_memory_protection(&info, label)?;

    let region_base = info.BaseAddress as usize as u64;
    let region_end = region_base
        .checked_add(info.RegionSize as u64)
        .ok_or_else(|| format!("{label} memory-region overflow"))?;
    let available = region_end
        .checked_sub(address)
        .ok_or_else(|| format!("{label} starts outside its memory region"))?;
    usize::try_from(available).map_err(|_| format!("{label} readable size does not fit usize"))
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

    let info = query_memory(process, address, label)?;
    validate_memory_protection(&info, label)?;
    let region_base = info.BaseAddress as usize as u64;
    let region_end = region_base
        .checked_add(info.RegionSize as u64)
        .ok_or_else(|| format!("{label} memory-region overflow"))?;
    if address < region_base || end > region_end {
        return Err(format!(
            "{label} range 0x{address:X}..0x{end:X} crosses a memory-region boundary"
        ));
    }
    Ok(())
}

fn query_memory(
    process: HANDLE,
    address: u64,
    label: &str,
) -> Result<MEMORY_BASIC_INFORMATION, String> {
    let address =
        usize::try_from(address).map_err(|_| format!("{label} address does not fit usize"))?;
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
    let queried = unsafe {
        VirtualQueryEx(
            process,
            address as *const _,
            &mut info,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if queried != size_of::<MEMORY_BASIC_INFORMATION>() {
        return Err(format!(
            "VirtualQueryEx failed for {label} at 0x{address:X}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(info)
}

fn validate_memory_protection(info: &MEMORY_BASIC_INFORMATION, label: &str) -> Result<(), String> {
    if info.State != MEM_COMMIT {
        return Err(format!("{label} is not committed memory"));
    }
    if info.Protect == 0 || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0 {
        return Err(format!(
            "{label} has unreadable protection 0x{:X}",
            info.Protect
        ));
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

fn read_i32(bytes: &[u8], offset: usize, label: &str) -> Result<i32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{label} is outside its buffer"))?;
    Ok(i32::from_le_bytes(
        value.try_into().map_err(|_| format!("Invalid {label}"))?,
    ))
}

fn read_i16(bytes: &[u8], offset: usize, label: &str) -> Result<i16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| format!("{label} is outside its buffer"))?;
    Ok(i16::from_le_bytes(
        value.try_into().map_err(|_| format!("Invalid {label}"))?,
    ))
}

fn read_u16(bytes: &[u8], offset: usize, label: &str) -> Result<u16, String> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| format!("{label} is outside its buffer"))?;
    Ok(u16::from_le_bytes(
        value.try_into().map_err(|_| format!("Invalid {label}"))?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize, label: &str) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let value = bytes
        .get(offset..end)
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
    use std::collections::HashMap;

    #[test]
    fn hash_traversal_rejects_cycles() {
        let nodes = HashMap::from([
            (
                0x10_000,
                HashNode {
                    next: 0x20_000,
                    data: 0x40_000,
                },
            ),
            (
                0x20_000,
                HashNode {
                    next: 0x10_000,
                    data: 0x50_000,
                },
            ),
        ]);
        let error = traverse_hash_nodes(&[0x10_000], 4, |address| {
            nodes
                .get(&address)
                .copied()
                .ok_or_else(|| "missing test node".to_string())
        })
        .expect_err("cycle must fail closed");
        assert!(error.contains("cycle detected"));
    }

    #[test]
    fn hash_traversal_enforces_node_limit() {
        let nodes = HashMap::from([
            (
                0x10_000,
                HashNode {
                    next: 0x20_000,
                    data: 0x40_000,
                },
            ),
            (
                0x20_000,
                HashNode {
                    next: 0x30_000,
                    data: 0x50_000,
                },
            ),
            (
                0x30_000,
                HashNode {
                    next: 0,
                    data: 0x60_000,
                },
            ),
        ]);
        let error = traverse_hash_nodes(&[0x10_000], 2, |address| {
            nodes
                .get(&address)
                .copied()
                .ok_or_else(|| "missing test node".to_string())
        })
        .expect_err("node limit must fail closed");
        assert!(error.contains("exceeded 2 nodes"));
    }

    #[test]
    fn vector_metadata_validates_count_capacity_and_pointer() {
        assert_eq!(validate_vector_metadata(4, 0x10_000, 4), Ok(4));
        assert!(validate_vector_metadata(-1, 0x10_000, 4).is_err());
        assert!(validate_vector_metadata(129, 0x10_000, 129).is_err());
        assert!(validate_vector_metadata(4, 0x10_000, 3).is_err());
        assert!(validate_vector_metadata(4, 0, 4).is_err());
    }

    #[test]
    fn scope_name_requires_a_bounded_terminator() {
        let mut terminated = vec![0_u8; TYPE_SCOPE_NAME_CAPACITY];
        terminated[..10].copy_from_slice(b"client.dll");
        assert_eq!(parse_scope_name(&terminated), Ok("client.dll".to_string()));

        let unterminated = vec![b'a'; TYPE_SCOPE_NAME_CAPACITY];
        assert!(parse_scope_name(&unterminated).is_err());
    }

    #[test]
    fn exact_scope_matching_rejects_prefixes_and_suffixes() {
        assert!(scope_name_matches("client", "client"));
        assert!(scope_name_matches("CLIENT.DLL", "client.dll"));
        assert!(!scope_name_matches("client.dll.old", "client.dll"));
        assert!(!scope_name_matches("some_client.dll", "client.dll"));
    }

    #[test]
    fn class_and_field_names_match_exactly() {
        assert!(class_name_matches(
            "C_CitadelPlayerPawn",
            "C_CitadelPlayerPawn"
        ));
        assert!(!class_name_matches(
            "C_CitadelPlayerPawnDerived",
            "C_CitadelPlayerPawn"
        ));
        assert!(field_name_matches(
            "m_nMapDistrictLocation",
            "m_nMapDistrictLocation"
        ));
        assert!(!field_name_matches(
            "m_nMapDistrictLocationOld",
            "m_nMapDistrictLocation"
        ));
    }

    #[test]
    fn class_fields_reject_invalid_count_and_null_pointer() {
        assert!(validate_class_fields(0, 0x10_000).is_err());
        assert!(validate_class_fields(-1, 0x10_000).is_err());
        assert!(validate_class_fields(1_025, 0x10_000).is_err());
        assert!(validate_class_fields(8, 0).is_err());
        assert_eq!(validate_class_fields(8, 0x10_000), Ok(8));
    }

    #[test]
    fn field_offset_rejects_negative_and_returns_valid_value() {
        assert!(validate_field_offset(-1, 0x4000).is_err());
        assert!(validate_field_offset(0x4000, 0x4000).is_err());
        assert_eq!(validate_field_offset(0x1234, 0x4000), Ok(0x1234));
    }
}
