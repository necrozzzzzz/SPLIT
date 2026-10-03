use std::{
    mem::{size_of, zeroed},
    sync::atomic::{AtomicI16, AtomicU32, Ordering},
    thread,
    time::{Duration, Instant},
};

#[cfg(debug_assertions)]
use std::sync::Mutex;

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        Memory::{VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS},
        Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
    },
};

use super::{camera, pawn, process};

const POLL_INTERVAL: Duration = Duration::from_millis(400);
const INITIAL_RESOLVE_RETRY_INTERVAL: Duration = Duration::from_millis(750);
const MIN_USER_ADDRESS: u64 = 0x1_0000;
const MAX_USER_ADDRESS: u64 = 0x0000_7FFF_FFFF_FFFF;
const NO_DISTRICT: i16 = -1;

static CURRENT_DISTRICT: AtomicI16 = AtomicI16::new(NO_DISTRICT);
static ACTIVE_PID: AtomicU32 = AtomicU32::new(0);
static MIDTOWN_ENTRY_GENERATION: AtomicU32 = AtomicU32::new(0);

#[cfg(debug_assertions)]
struct DistrictDebugTimeline {
    entered_at: Option<Instant>,
    schema_system: Option<u64>,
    worker_running: bool,
    district_offset: Option<u32>,
    controller: Option<u64>,
    pawn: Option<u64>,
    first_valid_raw_logged: bool,
    first_publication_logged: bool,
}

#[cfg(debug_assertions)]
static DEBUG_TIMELINE: Mutex<DistrictDebugTimeline> = Mutex::new(DistrictDebugTimeline {
    entered_at: None,
    schema_system: None,
    worker_running: false,
    district_offset: None,
    controller: None,
    pawn: None,
    first_valid_raw_logged: false,
    first_publication_logged: false,
});

#[derive(Debug)]
struct ResolutionRetry {
    next_attempt: Instant,
}

impl ResolutionRetry {
    fn immediate(now: Instant) -> Self {
        Self { next_attempt: now }
    }

    fn is_ready(&self, now: Instant) -> bool {
        now >= self.next_attempt
    }

    fn retry_after_failure(&mut self, now: Instant) {
        self.next_attempt = now + INITIAL_RESOLVE_RETRY_INTERVAL;
    }

    fn retry_immediately(&mut self, now: Instant) {
        self.next_attempt = now;
    }
}

#[cfg(debug_assertions)]
fn debug_log(timeline: &DistrictDebugTimeline, message: &str) {
    let elapsed = timeline
        .entered_at
        .map(|entered_at| entered_at.elapsed().as_millis())
        .unwrap_or_default();
    println!("[SPLIT][DistrictDebug] t={elapsed}ms {message}");
}

pub(crate) fn notify_midtown_entered() {
    MIDTOWN_ENTRY_GENERATION.fetch_add(1, Ordering::AcqRel);

    #[cfg(debug_assertions)]
    if let Ok(mut timeline) = DEBUG_TIMELINE.lock() {
        timeline.entered_at = Some(Instant::now());
        timeline.controller = None;
        timeline.pawn = None;
        timeline.first_valid_raw_logged = false;
        timeline.first_publication_logged = false;
        debug_log(&timeline, "entered dl_midtown");
        if let Some(schema_system) = timeline.schema_system {
            debug_log(
                &timeline,
                &format!("schema_ready=true address=0x{schema_system:016X}"),
            );
        }
        if let Some(offset) = timeline.district_offset {
            debug_log(
                &timeline,
                &format!("district_offset=0x{offset:X} available=true"),
            );
        }
        if timeline.worker_running {
            debug_log(&timeline, "district_worker_reactivation_requested=true");
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) fn debug_schema_ready(schema_system: u64) {
    if let Ok(mut timeline) = DEBUG_TIMELINE.lock() {
        if timeline.schema_system == Some(schema_system) {
            return;
        }
        timeline.schema_system = Some(schema_system);
        if timeline.entered_at.is_some() {
            debug_log(
                &timeline,
                &format!("schema_ready=true address=0x{schema_system:016X}"),
            );
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) fn debug_schema_offset(offset: u32) {
    if let Ok(mut timeline) = DEBUG_TIMELINE.lock() {
        if timeline.district_offset == Some(offset) {
            return;
        }
        timeline.district_offset = Some(offset);
        if timeline.entered_at.is_some() {
            debug_log(
                &timeline,
                &format!("district_offset=0x{offset:X} available=true"),
            );
        }
    }
}

#[cfg(debug_assertions)]
fn debug_worker_started() {
    if let Ok(mut timeline) = DEBUG_TIMELINE.lock() {
        if timeline.worker_running {
            return;
        }
        timeline.worker_running = true;
        if timeline.entered_at.is_some() {
            debug_log(&timeline, "district_worker_started=true");
        }
    }
}

#[cfg(debug_assertions)]
fn debug_worker_reactivated() {
    if let Ok(timeline) = DEBUG_TIMELINE.lock() {
        if timeline.entered_at.is_some() {
            debug_log(&timeline, "district_worker_reactivated=true");
        }
    }
}

#[cfg(debug_assertions)]
fn debug_worker_stopped() {
    if let Ok(mut timeline) = DEBUG_TIMELINE.lock() {
        timeline.worker_running = false;
    }
}

#[cfg(debug_assertions)]
pub(crate) fn debug_controller_found(controller: u64, scanned_slots: usize) {
    if let Ok(mut timeline) = DEBUG_TIMELINE.lock() {
        if timeline.controller == Some(controller) {
            return;
        }
        timeline.controller = Some(controller);
        if timeline.entered_at.is_some() {
            debug_log(
                &timeline,
                &format!("controller=0x{controller:016X} found=true scanned_slots={scanned_slots}"),
            );
        }
    }
}

#[cfg(debug_assertions)]
fn debug_resolution(resolution: pawn::PawnResolution) {
    if let Ok(mut timeline) = DEBUG_TIMELINE.lock() {
        if timeline.controller != Some(resolution.controller) {
            timeline.controller = Some(resolution.controller);
            if timeline.entered_at.is_some() {
                debug_log(
                    &timeline,
                    &format!("controller=0x{:016X} found=true", resolution.controller),
                );
            }
        }
        if timeline.pawn != Some(resolution.pawn) {
            timeline.pawn = Some(resolution.pawn);
            if timeline.entered_at.is_some() {
                debug_log(
                    &timeline,
                    &format!("pawn=0x{:016X} found=true", resolution.pawn),
                );
            }
        }
    }
}

#[cfg(debug_assertions)]
fn debug_first_valid_raw(raw: i8) {
    if let Ok(mut timeline) = DEBUG_TIMELINE.lock() {
        if timeline.entered_at.is_some() && !timeline.first_valid_raw_logged {
            timeline.first_valid_raw_logged = true;
            debug_log(&timeline, &format!("first_valid_raw={raw}"));
        }
    }
}

#[cfg(debug_assertions)]
fn debug_first_publication(raw: i8) {
    if let Ok(mut timeline) = DEBUG_TIMELINE.lock() {
        if timeline.entered_at.is_some() && !timeline.first_publication_logged {
            timeline.first_publication_logged = true;
            debug_log(
                &timeline,
                &format!("first_publication district_raw=Some({raw})"),
            );
        }
    }
}

struct ReadOnlyProcess(HANDLE);

impl Drop for ReadOnlyProcess {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[derive(Debug, Default)]
struct DistrictChangeDetector {
    previous: Option<i8>,
}

impl DistrictChangeDetector {
    fn observe(&mut self, value: Option<i8>) -> bool {
        if self.previous == value {
            return false;
        }
        self.previous = value;
        true
    }
}

pub(crate) const fn district_display_name(raw: i8) -> Option<&'static str> {
    match raw {
        1 => Some("Hidden King : Base"),
        2 => Some("Archmother : Base"),
        3 => Some("York : Docks"),
        4 => Some("Broadway : Times Square"),
        5 => Some("Greenwich : Central Park"),
        6 => Some("Haunted Lot"),
        7 => Some("Chinatown"),
        8 => Some("Plaza"),
        9 => Some("Theater"),
        10 => Some("Mid : Pit"),
        11 => Some("Canal : Park"),
        12 => Some("Canal : York"),
        13 => Some("York : Factory"),
        14 => Some("York : Stock Exchange"),
        15 => Some("Broadway : Uptown"),
        16 => Some("Broadway : Downtown"),
        17 => Some("Greenwich : Brownstones"),
        18 => Some("Greenwich : Campus"),
        19 => Some("Sunken Plaza"),
        _ => None,
    }
}

pub(crate) const fn district_asset_key(raw: i8) -> Option<&'static str> {
    match raw {
        1 => Some("hidden_king_base"),
        2 => Some("archmother_base"),
        3 => Some("york_docks"),
        4 => Some("broadway_times_square"),
        5 => Some("greenwich_central_park"),
        6 => Some("haunted_lot"),
        7 => Some("chinatown"),
        8 => Some("plaza"),
        9 => Some("theater"),
        10 => Some("mid_pit"),
        11 => Some("canal_park"),
        12 => Some("canal_york"),
        13 => Some("york_factory"),
        14 => Some("york_stock_exchange"),
        15 => Some("broadway_uptown"),
        16 => Some("broadway_downtown"),
        17 => Some("greenwich_brownstones"),
        18 => Some("greenwich_campus"),
        19 => Some("sunken_plaza"),
        _ => None,
    }
}

pub(crate) fn current_raw() -> Option<i8> {
    let value = CURRENT_DISTRICT.load(Ordering::Acquire);
    (value != NO_DISTRICT).then_some(value as i8)
}

fn publish_current(pid: u32, value: Option<i8>) {
    if ACTIVE_PID.load(Ordering::Acquire) == pid {
        CURRENT_DISTRICT.store(value.map_or(NO_DISTRICT, i16::from), Ordering::Release);
    }
}

fn is_explore_nyc_context(console: &super::console_phase::ConsolePhaseState) -> bool {
    !console.broadcast_active
        && console.server_kind == super::console_phase::ServerKind::Local
        && console.current_map.as_deref() == Some("dl_midtown")
}

pub(crate) fn start(
    pid: u32,
    local_flag_offset: u32,
    pawn_handle_offset: u32,
    identity_size: u32,
    district_offset: u32,
    initial_pawn: Option<pawn::PawnResolution>,
) -> Result<(), String> {
    ACTIVE_PID.store(pid, Ordering::Release);
    CURRENT_DISTRICT.store(NO_DISTRICT, Ordering::Release);
    thread::Builder::new()
        .name("split-district-probe".to_string())
        .spawn(move || {
            run(
                pid,
                local_flag_offset,
                pawn_handle_offset,
                identity_size,
                district_offset,
                initial_pawn,
            )
        })
        .map(|_| ())
        .map_err(|error| format!("Could not start district probe: {error}"))
}

fn run(
    pid: u32,
    local_flag_offset: u32,
    pawn_handle_offset: u32,
    identity_size: u32,
    district_offset: u32,
    mut resolution: Option<pawn::PawnResolution>,
) {
    let mut changes = DistrictChangeDetector::default();
    let started_at = Instant::now();
    let mut retry = ResolutionRetry::immediate(started_at);
    let mut observed_entry_generation = 0;

    #[cfg(debug_assertions)]
    {
        debug_worker_started();
        if let Some(current) = resolution {
            debug_resolution(current);
        }
    }

    while process::deadlock_pid() == Some(pid) {
        let now = Instant::now();
        let console = super::console_phase::snapshot();
        let explore_nyc_active = is_explore_nyc_context(&console);

        if !explore_nyc_active {
            resolution = None;
            retry.retry_immediately(now);

            if changes.observe(None) {
                publish_current(pid, None);
            }

            thread::sleep(POLL_INTERVAL);
            continue;
        }
        let entry_generation = MIDTOWN_ENTRY_GENERATION.load(Ordering::Acquire);
        if entry_generation != observed_entry_generation {
            observed_entry_generation = entry_generation;
            retry.retry_immediately(now);
            #[cfg(debug_assertions)]
            {
                debug_worker_reactivated();
                if let Some(current) = resolution {
                    debug_resolution(current);
                }
            }
        }

        if let Some(current) = resolution {
            let resolution_is_current =
                pawn::read_current_pawn_handle(pid, current.controller, pawn_handle_offset)
                    .is_ok_and(|handle| handle == current.pawn_handle);
            if !resolution_is_current {
                resolution = None;
                retry.retry_immediately(now);
                if changes.observe(None) {
                    publish_current(pid, None);
                }
            }
        }

        if resolution.is_none() && retry.is_ready(now) {
            match pawn::resolve_local_pawn(
                pid,
                local_flag_offset,
                pawn_handle_offset,
                identity_size,
            ) {
                Ok(current) => {
                    #[cfg(debug_assertions)]
                    debug_resolution(current);
                    resolution = Some(current);
                }
                Err(_) => retry.retry_after_failure(Instant::now()),
            }
        }

        if let Some(current) = resolution {
            match read_map_district_location(pid, current.pawn, district_offset) {
                Ok(Some(value)) if changes.observe(Some(value)) => {
                    #[cfg(debug_assertions)]
                    debug_first_valid_raw(value);
                    publish_current(pid, Some(value));
                    #[cfg(debug_assertions)]
                    debug_first_publication(value);
                    match district_display_name(value) {
                        Some(name) => {
                            println!("[SPLIT][District] raw location = {value} -> {name}")
                        }
                        None => println!("[SPLIT][District] raw location = {value} -> unknown"),
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => {
                    resolution = None;
                    if changes.observe(None) {
                        publish_current(pid, None);
                    }
                }
            }
        }

        thread::sleep(POLL_INTERVAL);
    }
    if ACTIVE_PID
        .compare_exchange(pid, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        CURRENT_DISTRICT.store(NO_DISTRICT, Ordering::Release);
    }
    #[cfg(debug_assertions)]
    debug_worker_stopped();
}

pub(crate) fn read_map_district_location(
    pid: u32,
    pawn_address: u64,
    schema_offset: u32,
) -> Result<Option<i8>, String> {
    let address = district_address(pawn_address, schema_offset)?;
    let process = open_process_read_only(pid)?;
    validate_readable_range(process.0, address, 1, "map district location")?;
    let bytes = camera::read_bytes(
        process.0,
        usize::try_from(address)
            .map_err(|_| "Map district address does not fit usize".to_string())?,
        1,
    )?;
    let raw = *bytes
        .first()
        .ok_or_else(|| "Map district read returned no data".to_string())? as i8;
    Ok(validate_district_value(raw))
}

fn district_address(pawn_address: u64, schema_offset: u32) -> Result<u64, String> {
    pawn_address
        .checked_add(schema_offset as u64)
        .ok_or_else(|| "Map district address overflow".to_string())
}

fn validate_district_value(value: i8) -> Option<i8> {
    (value >= 0).then_some(value)
}

fn open_process_read_only(pid: u32) -> Result<ReadOnlyProcess, String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        return Err(format!(
            "Could not open Deadlock PID {pid} for district reading: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(ReadOnlyProcess(handle))
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
    if !(MIN_USER_ADDRESS..=MAX_USER_ADDRESS).contains(&address) {
        return Err(format!(
            "{label} is not a canonical user address: 0x{address:X}"
        ));
    }
    let end = address
        .checked_add(size as u64)
        .ok_or_else(|| format!("{label} range overflow"))?;
    if end == 0 || end - 1 > MAX_USER_ADDRESS {
        return Err(format!("{label} range is outside canonical user memory"));
    }

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
        .ok_or_else(|| format!("{label} memory-region overflow"))?;
    if address < region_base || end > region_end {
        return Err(format!("{label} crosses a memory-region boundary"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_resolution_attempt_is_immediate() {
        let entered_at = Instant::now();
        let retry = ResolutionRetry::immediate(entered_at);

        assert!(retry.is_ready(entered_at));
    }

    #[test]
    fn failed_initial_resolution_retries_without_a_multi_second_cooldown() {
        let attempted_at = Instant::now();
        let mut retry = ResolutionRetry::immediate(attempted_at);
        retry.retry_after_failure(attempted_at);

        assert!(!retry
            .is_ready(attempted_at + INITIAL_RESOLVE_RETRY_INTERVAL - Duration::from_millis(1)));
        assert!(retry.is_ready(attempted_at + INITIAL_RESOLVE_RETRY_INTERVAL));
        assert!(INITIAL_RESOLVE_RETRY_INTERVAL < Duration::from_secs(1));
    }

    #[test]
    fn pawn_invalidation_makes_resolution_immediately_retryable() {
        let attempted_at = Instant::now();
        let invalidated_at = attempted_at + Duration::from_millis(50);
        let mut retry = ResolutionRetry::immediate(attempted_at);
        retry.retry_after_failure(attempted_at);
        retry.retry_immediately(invalidated_at);

        assert!(retry.is_ready(invalidated_at));
    }

    #[test]
    fn acquired_district_polling_interval_remains_four_hundred_milliseconds() {
        assert_eq!(POLL_INTERVAL, Duration::from_millis(400));
    }

    #[test]
    fn hltv_broadcast_never_activates_the_explore_nyc_probe() {
        let mut console = super::super::console_phase::ConsolePhaseState::default();
        console.server_kind = super::super::console_phase::ServerKind::Local;
        console.current_map = Some("dl_midtown".to_string());
        assert!(is_explore_nyc_context(&console));

        console.broadcast_active = true;
        assert!(!is_explore_nyc_context(&console));
    }

    #[test]
    fn pawn_plus_offset_rejects_overflow() {
        assert!(district_address(u64::MAX - 3, 4).is_err());
        assert_eq!(district_address(0x1000, 0x22), Ok(0x1022));
    }

    #[test]
    fn negative_i8_is_rejected_and_valid_value_is_accepted() {
        assert_eq!(validate_district_value(-1), None);
        assert_eq!(validate_district_value(i8::MIN), None);
        assert_eq!(validate_district_value(0), Some(0));
        assert_eq!(validate_district_value(127), Some(127));
    }

    #[test]
    fn district_change_detector_emits_only_on_change() {
        let mut detector = DistrictChangeDetector::default();
        assert!(detector.observe(Some(3)));
        assert!(!detector.observe(Some(3)));
        assert!(detector.observe(Some(8)));
        assert!(!detector.observe(Some(8)));
        assert!(detector.observe(None));
        assert!(!detector.observe(None));
    }

    #[test]
    fn observed_district_mapping_is_exact_and_unknowns_are_not_guessed() {
        assert_eq!(district_display_name(1), Some("Hidden King : Base"));
        assert_eq!(district_display_name(3), Some("York : Docks"));
        assert_eq!(district_display_name(10), Some("Mid : Pit"));
        assert_eq!(district_display_name(11), Some("Canal : Park"));
        assert_eq!(district_display_name(12), Some("Canal : York"));
        assert_eq!(district_display_name(18), Some("Greenwich : Campus"));
        assert_eq!(district_display_name(19), Some("Sunken Plaza"));
        assert_eq!(district_display_name(0), None);
        assert_eq!(district_display_name(127), None);
    }

    #[test]
    fn district_asset_mapping_is_exact_and_unknowns_use_the_fallback() {
        assert_eq!(district_asset_key(1), Some("hidden_king_base"));
        assert_eq!(district_asset_key(3), Some("york_docks"));
        assert_eq!(district_asset_key(10), Some("mid_pit"));
        assert_eq!(district_asset_key(19), Some("sunken_plaza"));
        assert_eq!(district_asset_key(0), None);
        assert_eq!(district_asset_key(20), None);
    }
}
