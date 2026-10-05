use std::collections::HashSet;
use std::{thread, time::Duration};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
};

use super::camera;

const CREATE_INTERFACE_EXPORT: &str = "CreateInterface";
const SCHEMA_SYSTEM_MODULE: &str = "schemasystem.dll";
const SCHEMA_SYSTEM_INTERFACE: &str = "SchemaSystem_001";

const MAX_MODULE_SIZE: usize = 512 * 1024 * 1024;
const MAX_EXPORT_NAMES: usize = 65_536;
const MAX_INTERFACE_NAME: usize = 128;
const MAX_INTERFACE_REGISTRATIONS: usize = 1_024;
const INTERFACE_REG_SIZE: usize = 24;
const MODULE_DISCOVERY_ATTEMPTS: usize = 40;
const MODULE_DISCOVERY_RETRY_DELAY: Duration = Duration::from_millis(250);
const SCHEMA_INITIALIZATION_ATTEMPTS: usize = 21;
const SCHEMA_INITIALIZATION_RETRY_DELAY: Duration = Duration::from_millis(250);

const PE_SIGNATURE: u32 = 0x0000_4550;
const PE32_PLUS_MAGIC: u16 = 0x020B;

/// Finds a Source 2 interface without executing code in the target process.
///
/// The resolver reads the loaded module as a PE image, resolves the
/// `CreateInterface` export, then follows the x64 RIP-relative references used
/// by Source 2's interface registry and its factory functions.
pub(crate) fn find_interface(
    pid: u32,
    module_name: &str,
    interface_name: &str,
) -> Result<Option<u64>, String> {
    find_interface_with_module_retry(
        pid,
        module_name,
        interface_name,
        MODULE_DISCOVERY_ATTEMPTS,
        MODULE_DISCOVERY_RETRY_DELAY,
    )
}

fn find_interface_with_module_retry(
    pid: u32,
    module_name: &str,
    interface_name: &str,
    module_attempts: usize,
    module_retry_delay: Duration,
) -> Result<Option<u64>, String> {
    if interface_name.is_empty() || interface_name.len() > MAX_INTERFACE_NAME {
        return Err("Interface name length is invalid".to_string());
    }

    let (module_base, module_size) = if module_attempts == 1 {
        camera::find_module(pid, module_name)?
    } else {
        camera::find_module_with_retry(pid, module_name, module_attempts, module_retry_delay)?
    };
    if !(0x1000..=MAX_MODULE_SIZE).contains(&module_size) {
        return Err(format!(
            "{module_name} has a suspicious image size: 0x{module_size:X}"
        ));
    }

    let process = open_process_read_only(pid)?;
    let image_result = camera::read_bytes(process, module_base, module_size);
    unsafe {
        let _ = CloseHandle(process);
    }

    let image = image_result?;
    if image.len() != module_size {
        return Err(format!(
            "Incomplete {module_name} image read: expected 0x{module_size:X}, got 0x{:X}",
            image.len()
        ));
    }

    resolve_interface_from_image(&image, module_base as u64, interface_name)
}

#[derive(Debug, PartialEq, Eq)]
enum InitializationRetryOutcome {
    Initialized { attempts: usize },
    Exhausted { attempts: usize, last_error: String },
    SessionEnded { attempts: usize },
}

fn retry_initialization(
    max_attempts: usize,
    mut session_is_valid: impl FnMut() -> bool,
    mut initialize: impl FnMut() -> Result<(), String>,
    mut wait: impl FnMut(),
) -> InitializationRetryOutcome {
    let mut last_error = "initialization was not attempted".to_string();

    for attempt in 1..=max_attempts {
        if !session_is_valid() {
            return InitializationRetryOutcome::SessionEnded {
                attempts: attempt - 1,
            };
        }

        match initialize() {
            Ok(()) => {
                return InitializationRetryOutcome::Initialized { attempts: attempt };
            }
            Err(error) => {
                if attempt == 1 || attempt == max_attempts || attempt % 4 == 0 {
                    eprintln!(
                        "[SPLIT][Schema] district initialization attempt {attempt} failed: {error}"
                    );
                }
                last_error = error;
            }
        }

        if attempt < max_attempts {
            if !session_is_valid() {
                return InitializationRetryOutcome::SessionEnded { attempts: attempt };
            }
            wait();
        }
    }

    InitializationRetryOutcome::Exhausted {
        attempts: max_attempts,
        last_error,
    }
}

pub(crate) fn initialize_schema_runtime(pid: u32) {
    let spawn_result = thread::Builder::new()
        .name("split-schema-interface-probe".to_string())
        .spawn(move || initialize_schema_runtime_now(pid));

    if let Err(error) = spawn_result {
        eprintln!(
            "[SPLIT][Interface] {SCHEMA_SYSTEM_INTERFACE} unavailable: could not start probe: {error}"
        );
    }
}

fn initialize_schema_runtime_now(pid: u32) {
    let outcome = retry_initialization(
        SCHEMA_INITIALIZATION_ATTEMPTS,
        || super::process::deadlock_pid() == Some(pid),
        || initialize_schema_runtime_once(pid),
        || thread::sleep(SCHEMA_INITIALIZATION_RETRY_DELAY),
    );

    match outcome {
        InitializationRetryOutcome::Initialized { attempts } => {
            println!(
                "[SPLIT][Schema] district runtime initialized after {attempts} attempt(s)"
            );
        }
        InitializationRetryOutcome::Exhausted {
            attempts,
            last_error,
        } => eprintln!(
            "[SPLIT][Schema] district initialization exhausted after {attempts} attempts: {last_error}"
        ),
        InitializationRetryOutcome::SessionEnded { attempts } => println!(
            "[SPLIT][Schema] district initialization stopped after {attempts} attempt(s): Deadlock session ended"
        ),
    }
}

fn initialize_schema_runtime_once(pid: u32) -> Result<(), String> {
    let address = find_interface_with_module_retry(
        pid,
        SCHEMA_SYSTEM_MODULE,
        SCHEMA_SYSTEM_INTERFACE,
        1,
        Duration::ZERO,
    )?
    .ok_or_else(|| format!("exact interface {SCHEMA_SYSTEM_INTERFACE} was not found"))?;

    #[cfg(debug_assertions)]
    super::district::debug_schema_ready(address);

    super::schema::initialize_client_runtime(pid, address)?;
    println!("[SPLIT][Interface] {SCHEMA_SYSTEM_INTERFACE} = 0x{address:016X}");
    Ok(())
}

fn open_process_read_only(pid: u32) -> Result<HANDLE, String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };

    if process.is_null() {
        return Err(format!(
            "Could not open Deadlock for interface discovery: {}",
            std::io::Error::last_os_error()
        ));
    }

    Ok(process)
}

fn resolve_interface_from_image(
    image: &[u8],
    module_base: u64,
    interface_name: &str,
) -> Result<Option<u64>, String> {
    let create_interface_rva = find_export_rva(image, CREATE_INTERFACE_EXPORT)?
        .ok_or_else(|| "CreateInterface export was not found".to_string())?;
    let create_interface = checked_runtime_address(
        image,
        module_base,
        create_interface_rva as u64,
        7,
        "CreateInterface export",
    )?;

    // Source 2's exported function starts with a 64-bit MOV from [RIP+rel32].
    // The destination register is build-dependent.
    // The target is the module-local global holding InterfaceReg*.
    let list_global =
        decode_rip_relative(image, module_base, create_interface, RipRelativeOpcode::Mov)?;
    let list_head = read_address(image, module_base, list_global, "InterfaceReg list head")?;

    walk_interface_regs(
        image,
        module_base,
        list_head,
        interface_name,
        MAX_INTERFACE_REGISTRATIONS,
    )
}

fn walk_interface_regs(
    image: &[u8],
    module_base: u64,
    mut registration: u64,
    interface_name: &str,
    traversal_limit: usize,
) -> Result<Option<u64>, String> {
    if traversal_limit == 0 {
        return Err("InterfaceReg traversal limit is zero".to_string());
    }

    let mut visited = HashSet::new();

    while registration != 0 {
        if !visited.insert(registration) {
            return Err(format!("InterfaceReg cycle detected at 0x{registration:X}"));
        }
        if visited.len() > traversal_limit {
            return Err(format!(
                "InterfaceReg traversal exceeded {traversal_limit} entries"
            ));
        }

        checked_address_offset(
            image,
            module_base,
            registration,
            INTERFACE_REG_SIZE,
            "InterfaceReg node",
        )?;

        // Source 2 x64 InterfaceReg layout:
        // +0x00 create_fn, +0x08 name, +0x10 next.
        let create_fn = read_address(image, module_base, registration, "InterfaceReg create_fn")?;
        let name_address = read_address(
            image,
            module_base,
            checked_add(registration, 8, "InterfaceReg name field")?,
            "InterfaceReg name",
        )?;
        let next = read_address(
            image,
            module_base,
            checked_add(registration, 16, "InterfaceReg next field")?,
            "InterfaceReg next",
        )?;

        checked_address_offset(image, module_base, create_fn, 7, "InterfaceReg create_fn")?;
        let name = read_c_string(
            image,
            module_base,
            name_address,
            MAX_INTERFACE_NAME,
            "InterfaceReg name",
        )?;

        if name == interface_name {
            // Source 2's per-interface factory is a 64-bit LEA from
            // [RIP+rel32]. The destination register is build-dependent.
            // It returns the address of the static interface instance.
            return decode_rip_relative(image, module_base, create_fn, RipRelativeOpcode::Lea)
                .map(Some);
        }

        if next != 0 {
            checked_address_offset(
                image,
                module_base,
                next,
                INTERFACE_REG_SIZE,
                "next InterfaceReg node",
            )?;
        }
        registration = next;
    }

    Ok(None)
}

fn find_export_rva(image: &[u8], export_name: &str) -> Result<Option<u32>, String> {
    let pe_offset = read_u32(image, 0x3C, "DOS e_lfanew")? as usize;
    if !(0x40..=0x4000).contains(&pe_offset) {
        return Err(format!("Suspicious PE header offset: 0x{pe_offset:X}"));
    }
    if read_u32(image, pe_offset, "PE signature")? != PE_SIGNATURE {
        return Err("Invalid PE signature".to_string());
    }

    let file_header = pe_offset
        .checked_add(4)
        .ok_or_else(|| "PE file-header offset overflow".to_string())?;
    let optional_header_size = read_u16(image, file_header + 16, "optional header size")? as usize;
    let optional_header = file_header
        .checked_add(20)
        .ok_or_else(|| "PE optional-header offset overflow".to_string())?;
    checked_slice(
        image,
        optional_header,
        optional_header_size,
        "PE optional header",
    )?;
    if optional_header_size < 120 {
        return Err("PE optional header is too small for export data".to_string());
    }
    if read_u16(image, optional_header, "PE optional-header magic")? != PE32_PLUS_MAGIC {
        return Err("schemasystem.dll is not a PE32+ image".to_string());
    }
    if read_u32(image, optional_header + 108, "PE data-directory count")? == 0 {
        return Ok(None);
    }

    let export_rva = read_u32(image, optional_header + 112, "export directory RVA")?;
    let export_size = read_u32(image, optional_header + 116, "export directory size")?;
    if export_rva == 0 || export_size == 0 {
        return Ok(None);
    }
    let export_offset =
        checked_rva_range(image, export_rva, export_size as usize, "export directory")?;
    checked_slice(image, export_offset, 40, "IMAGE_EXPORT_DIRECTORY")?;

    let function_count = read_u32(image, export_offset + 20, "export function count")? as usize;
    let name_count = read_u32(image, export_offset + 24, "export name count")? as usize;
    if name_count > MAX_EXPORT_NAMES || name_count > function_count {
        return Err(format!("Suspicious PE export-name count: {name_count}"));
    }

    let functions_rva = read_u32(image, export_offset + 28, "export function table")?;
    let names_rva = read_u32(image, export_offset + 32, "export name table")?;
    let ordinals_rva = read_u32(image, export_offset + 36, "export ordinal table")?;
    let functions = checked_rva_range(
        image,
        functions_rva,
        checked_mul(function_count, 4, "export function table size")?,
        "export function table",
    )?;
    let names = checked_rva_range(
        image,
        names_rva,
        checked_mul(name_count, 4, "export name table size")?,
        "export name table",
    )?;
    let ordinals = checked_rva_range(
        image,
        ordinals_rva,
        checked_mul(name_count, 2, "export ordinal table size")?,
        "export ordinal table",
    )?;

    for index in 0..name_count {
        let name_rva = read_u32(image, names + index * 4, "export name RVA")?;
        let name = read_rva_c_string(image, name_rva, MAX_INTERFACE_NAME, "export name")?;
        if name != export_name {
            continue;
        }

        let ordinal = read_u16(image, ordinals + index * 2, "export ordinal")? as usize;
        if ordinal >= function_count {
            return Err(format!("Export ordinal {ordinal} is out of range"));
        }
        let function_rva = read_u32(image, functions + ordinal * 4, "export function RVA")?;
        if function_rva == 0 {
            return Err(format!("{export_name} has a null export RVA"));
        }

        let export_end = export_rva
            .checked_add(export_size)
            .ok_or_else(|| "Export-directory range overflow".to_string())?;
        if (export_rva..export_end).contains(&function_rva) {
            return Err(format!("Forwarded {export_name} exports are unsupported"));
        }
        checked_rva_range(image, function_rva, 7, export_name)?;
        return Ok(Some(function_rva));
    }

    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RipRelativeOpcode {
    Mov,
    Lea,
}

impl RipRelativeOpcode {
    const fn mnemonic(self) -> &'static str {
        match self {
            Self::Mov => "MOV",
            Self::Lea => "LEA",
        }
    }
}

fn decode_rip_relative(
    image: &[u8],
    module_base: u64,
    instruction: u64,
    expected_opcode: RipRelativeOpcode,
) -> Result<u64, String> {
    let offset = checked_address_offset(
        image,
        module_base,
        instruction,
        7,
        "RIP-relative instruction",
    )?;
    let rex = image[offset];
    if rex & 0xF8 != 0x48 {
        return Err(format!(
            "Unsupported REX prefix at 0x{instruction:X}: 0x{rex:02X}; REX.W is required"
        ));
    }

    let opcode = match image[offset + 1] {
        0x8B => RipRelativeOpcode::Mov,
        0x8D => RipRelativeOpcode::Lea,
        byte => {
            return Err(format!(
                "Unsupported RIP-relative opcode at 0x{instruction:X}: 0x{byte:02X}"
            ));
        }
    };
    if opcode != expected_opcode {
        return Err(format!(
            "Expected {} at 0x{instruction:X}, found {}",
            expected_opcode.mnemonic(),
            opcode.mnemonic()
        ));
    }

    let modrm = image[offset + 2];
    if modrm & 0xC7 != 0x05 {
        return Err(format!(
            "Instruction at 0x{instruction:X} is not RIP-relative: ModRM=0x{modrm:02X}"
        ));
    }

    let displacement = i32::from_le_bytes(
        image[offset + 3..offset + 7]
            .try_into()
            .map_err(|_| "Invalid RIP displacement".to_string())?,
    );
    let instruction_end = instruction
        .checked_add(7)
        .ok_or_else(|| "RIP instruction address overflow".to_string())?;
    let target = (instruction_end as i128) + (displacement as i128);
    if target < 0 || target > u64::MAX as i128 {
        return Err("RIP-relative target overflow".to_string());
    }
    let target = target as u64;
    checked_address_offset(image, module_base, target, 1, "RIP-relative target")?;
    Ok(target)
}

fn read_address(image: &[u8], module_base: u64, address: u64, label: &str) -> Result<u64, String> {
    let offset = checked_address_offset(image, module_base, address, 8, label)?;
    read_u64(image, offset, label)
}

fn read_c_string<'a>(
    image: &'a [u8],
    module_base: u64,
    address: u64,
    max_length: usize,
    label: &str,
) -> Result<&'a str, String> {
    let offset = checked_address_offset(image, module_base, address, 1, label)?;
    read_offset_c_string(image, offset, max_length, label)
}

fn read_rva_c_string<'a>(
    image: &'a [u8],
    rva: u32,
    max_length: usize,
    label: &str,
) -> Result<&'a str, String> {
    let offset = checked_rva_range(image, rva, 1, label)?;
    read_offset_c_string(image, offset, max_length, label)
}

fn read_offset_c_string<'a>(
    image: &'a [u8],
    offset: usize,
    max_length: usize,
    label: &str,
) -> Result<&'a str, String> {
    let available = image.len().saturating_sub(offset).min(max_length + 1);
    let bytes = checked_slice(image, offset, available, label)?;
    let terminator = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| format!("{label} is not terminated within {max_length} bytes"))?;
    if terminator > max_length {
        return Err(format!("{label} exceeds {max_length} bytes"));
    }
    std::str::from_utf8(&bytes[..terminator]).map_err(|_| format!("{label} is not valid UTF-8"))
}

fn checked_runtime_address(
    image: &[u8],
    module_base: u64,
    rva: u64,
    size: usize,
    label: &str,
) -> Result<u64, String> {
    let address = module_base
        .checked_add(rva)
        .ok_or_else(|| format!("{label} address overflow"))?;
    checked_address_offset(image, module_base, address, size, label)?;
    Ok(address)
}

fn checked_address_offset(
    image: &[u8],
    module_base: u64,
    address: u64,
    size: usize,
    label: &str,
) -> Result<usize, String> {
    let rva = address
        .checked_sub(module_base)
        .ok_or_else(|| format!("{label} points below the module: 0x{address:X}"))?;
    let offset = usize::try_from(rva).map_err(|_| format!("{label} RVA does not fit usize"))?;
    checked_slice(image, offset, size, label)?;
    Ok(offset)
}

fn checked_rva_range(image: &[u8], rva: u32, size: usize, label: &str) -> Result<usize, String> {
    let offset = rva as usize;
    checked_slice(image, offset, size, label)?;
    Ok(offset)
}

fn checked_slice<'a>(
    image: &'a [u8],
    offset: usize,
    size: usize,
    label: &str,
) -> Result<&'a [u8], String> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| format!("{label} range overflow"))?;
    image
        .get(offset..end)
        .ok_or_else(|| format!("{label} is outside the module image"))
}

fn checked_add(value: u64, addend: u64, label: &str) -> Result<u64, String> {
    value
        .checked_add(addend)
        .ok_or_else(|| format!("{label} address overflow"))
}

fn checked_mul(value: usize, multiplier: usize, label: &str) -> Result<usize, String> {
    value
        .checked_mul(multiplier)
        .ok_or_else(|| format!("{label} overflow"))
}

fn read_u16(image: &[u8], offset: usize, label: &str) -> Result<u16, String> {
    let bytes = checked_slice(image, offset, 2, label)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(image: &[u8], offset: usize, label: &str) -> Result<u32, String> {
    let bytes = checked_slice(image, offset, 4, label)?;
    Ok(u32::from_le_bytes(
        bytes.try_into().map_err(|_| format!("Invalid {label}"))?,
    ))
}

fn read_u64(image: &[u8], offset: usize, label: &str) -> Result<u64, String> {
    let bytes = checked_slice(image, offset, 8, label)?;
    Ok(u64::from_le_bytes(
        bytes.try_into().map_err(|_| format!("Invalid {label}"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const BASE: u64 = 0x0000_7FF6_1000_0000;

    #[test]
    fn initialization_retry_succeeds_on_first_attempt() {
        let attempts = Cell::new(0);
        let waits = Cell::new(0);

        let outcome = retry_initialization(
            4,
            || true,
            || {
                attempts.set(attempts.get() + 1);
                Ok(())
            },
            || waits.set(waits.get() + 1),
        );

        assert_eq!(
            outcome,
            InitializationRetryOutcome::Initialized { attempts: 1 }
        );
        assert_eq!(attempts.get(), 1);
        assert_eq!(waits.get(), 0);
    }

    #[test]
    fn initialization_retry_recovers_after_transient_failure() {
        let attempts = Cell::new(0);
        let waits = Cell::new(0);

        let outcome = retry_initialization(
            4,
            || true,
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    Err("schema not ready".to_string())
                } else {
                    Ok(())
                }
            },
            || waits.set(waits.get() + 1),
        );

        assert_eq!(
            outcome,
            InitializationRetryOutcome::Initialized { attempts: 2 }
        );
        assert_eq!(attempts.get(), 2);
        assert_eq!(waits.get(), 1);
    }

    #[test]
    fn initialization_retry_reports_exhaustion() {
        let attempts = Cell::new(0);
        let waits = Cell::new(0);

        let outcome = retry_initialization(
            3,
            || true,
            || {
                attempts.set(attempts.get() + 1);
                Err(format!("failure {}", attempts.get()))
            },
            || waits.set(waits.get() + 1),
        );

        assert_eq!(
            outcome,
            InitializationRetryOutcome::Exhausted {
                attempts: 3,
                last_error: "failure 3".to_string(),
            }
        );
        assert_eq!(attempts.get(), 3);
        assert_eq!(waits.get(), 2);
    }

    #[test]
    fn initialization_retry_stops_when_session_ends() {
        let session_valid = Cell::new(true);
        let attempts = Cell::new(0);
        let waits = Cell::new(0);

        let outcome = retry_initialization(
            4,
            || session_valid.get(),
            || {
                attempts.set(attempts.get() + 1);
                session_valid.set(false);
                Err("process exited".to_string())
            },
            || waits.set(waits.get() + 1),
        );

        assert_eq!(
            outcome,
            InitializationRetryOutcome::SessionEnded { attempts: 1 }
        );
        assert_eq!(attempts.get(), 1);
        assert_eq!(waits.get(), 0);
    }

    fn put_u64(image: &mut [u8], offset: usize, value: u64) {
        image[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn put_rip_instruction(image: &mut [u8], offset: usize, opcode: [u8; 3], target_offset: usize) {
        image[offset..offset + 3].copy_from_slice(&opcode);
        let displacement = target_offset as i64 - (offset + 7) as i64;
        image[offset + 3..offset + 7].copy_from_slice(&(displacement as i32).to_le_bytes());
    }

    fn put_registration(
        image: &mut [u8],
        node_offset: usize,
        create_fn_offset: usize,
        name_offset: usize,
        name: &str,
        next_offset: Option<usize>,
        instance_offset: usize,
    ) {
        put_u64(image, node_offset, BASE + create_fn_offset as u64);
        put_u64(image, node_offset + 8, BASE + name_offset as u64);
        put_u64(
            image,
            node_offset + 16,
            next_offset.map_or(0, |offset| BASE + offset as u64),
        );
        image[name_offset..name_offset + name.len()].copy_from_slice(name.as_bytes());
        image[name_offset + name.len()] = 0;
        put_rip_instruction(image, create_fn_offset, [0x48, 0x8D, 0x05], instance_offset);
    }

    #[test]
    fn decodes_mov_and_lea_with_valid_rex_and_destination_registers() {
        let mut image = vec![0_u8; 0x180];
        put_rip_instruction(&mut image, 0x20, [0x48, 0x8B, 0x0D], 0x80);
        put_rip_instruction(&mut image, 0x40, [0x4C, 0x8B, 0x0D], 0xA0);
        put_rip_instruction(&mut image, 0x80, [0x48, 0x8D, 0x05], 0x10);
        put_rip_instruction(&mut image, 0xA0, [0x4D, 0x8D, 0x1D], 0x120);

        assert_eq!(
            decode_rip_relative(&image, BASE, BASE + 0x20, RipRelativeOpcode::Mov),
            Ok(BASE + 0x80)
        );
        assert_eq!(
            decode_rip_relative(&image, BASE, BASE + 0x40, RipRelativeOpcode::Mov),
            Ok(BASE + 0xA0)
        );
        assert_eq!(
            decode_rip_relative(&image, BASE, BASE + 0x80, RipRelativeOpcode::Lea),
            Ok(BASE + 0x10)
        );
        assert_eq!(
            decode_rip_relative(&image, BASE, BASE + 0xA0, RipRelativeOpcode::Lea),
            Ok(BASE + 0x120)
        );
    }

    #[test]
    fn rejects_non_rip_modrm_and_unsupported_opcode() {
        let mut image = vec![0_u8; 0x100];
        put_rip_instruction(&mut image, 0x20, [0x48, 0x8B, 0x08], 0x80);
        put_rip_instruction(&mut image, 0x40, [0x48, 0x89, 0x0D], 0x80);

        let modrm_error = decode_rip_relative(&image, BASE, BASE + 0x20, RipRelativeOpcode::Mov)
            .expect_err("non-RIP ModRM must be rejected");
        assert!(modrm_error.contains("not RIP-relative"));

        let opcode_error = decode_rip_relative(&image, BASE, BASE + 0x40, RipRelativeOpcode::Mov)
            .expect_err("unsupported opcode must be rejected");
        assert!(opcode_error.contains("Unsupported RIP-relative opcode"));
    }

    #[test]
    fn rejects_an_artificial_interface_reg_cycle() {
        let mut image = vec![0_u8; 0x300];
        put_registration(
            &mut image,
            0x40,
            0x180,
            0x200,
            "First_001",
            Some(0x80),
            0x280,
        );
        put_registration(
            &mut image,
            0x80,
            0x190,
            0x220,
            "Second_001",
            Some(0x40),
            0x290,
        );

        let error = walk_interface_regs(&image, BASE, BASE + 0x40, "Missing_001", 8)
            .expect_err("cycle must fail closed");
        assert!(error.contains("cycle detected"));
    }

    #[test]
    fn enforces_the_interface_reg_traversal_limit() {
        let mut image = vec![0_u8; 0x400];
        put_registration(
            &mut image,
            0x40,
            0x200,
            0x280,
            "First_001",
            Some(0x80),
            0x300,
        );
        put_registration(
            &mut image,
            0x80,
            0x210,
            0x2A0,
            "Second_001",
            Some(0xC0),
            0x310,
        );
        put_registration(&mut image, 0xC0, 0x220, 0x2C0, "Third_001", None, 0x320);

        let error = walk_interface_regs(&image, BASE, BASE + 0x40, "Missing_001", 2)
            .expect_err("traversal past the limit must fail closed");
        assert!(error.contains("exceeded 2 entries"));
    }

    #[test]
    fn matches_only_the_exact_interface_name() {
        let mut image = vec![0_u8; 0x500];
        put_registration(
            &mut image,
            0x40,
            0x200,
            0x300,
            "SchemaSystem_001_extra",
            Some(0x80),
            0x400,
        );
        put_registration(
            &mut image,
            0x80,
            0x220,
            0x340,
            "SchemaSystem_001",
            None,
            0x440,
        );

        assert_eq!(
            walk_interface_regs(&image, BASE, BASE + 0x40, "SchemaSystem_001", 8),
            Ok(Some(BASE + 0x440))
        );
    }
}
