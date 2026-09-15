use std::{
    fs,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::null_mut,
};

use windows_sys::{
    core::BOOL,
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
            ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetClientRect, GetWindowThreadProcessId, IsWindowVisible, PrintWindow,
        },
    },
};

struct WindowSearch {
    pid: u32,
    hwnd: HWND,
    area: i64,
}

unsafe extern "system" fn enum_window_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let search = &mut *(lparam as *mut WindowSearch);

    if IsWindowVisible(hwnd) == 0 {
        return 1;
    }

    let mut pid = 0u32;

    GetWindowThreadProcessId(hwnd, &mut pid);

    if pid != search.pid {
        return 1;
    }

    let mut rect: RECT = zeroed();

    if GetClientRect(hwnd, &mut rect) == 0 {
        return 1;
    }

    let width = rect.right - rect.left;

    let height = rect.bottom - rect.top;

    if width <= 0 || height <= 0 {
        return 1;
    }

    let area = i64::from(width) * i64::from(height);

    /*
     * Un process peut posséder plusieurs
     * fenêtres visibles.
     *
     * On garde la plus grande :
     * ce sera normalement la vraie fenêtre
     * de rendu de Deadlock.
     */
    if area > search.area {
        search.hwnd = hwnd;
        search.area = area;
    }

    1
}

fn find_deadlock_window() -> Result<HWND, String> {
    let pid =
        super::process::deadlock_pid().ok_or_else(|| "Deadlock is not running.".to_string())?;

    let mut search = WindowSearch {
        pid,
        hwnd: null_mut(),
        area: 0,
    };

    unsafe {
        EnumWindows(
            Some(enum_window_callback),
            &mut search as *mut WindowSearch as LPARAM,
        );
    }

    if search.hwnd.is_null() {
        return Err("Could not find a visible Deadlock window.".to_string());
    }

    Ok(search.hwnd)
}

fn output_path() -> Result<PathBuf, String> {
    let appdata =
        std::env::var_os("APPDATA").ok_or_else(|| "APPDATA is unavailable.".to_string())?;

    let directory = PathBuf::from(appdata).join("SPLIT");

    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create SPLIT directory: {error}"))?;

    Ok(directory.join("test-capture.bmp"))
}

fn write_bmp(path: &PathBuf, width: i32, height: i32, pixels: &[u8]) -> Result<(), String> {
    /*
     * BMP très simple :
     *
     * 14 bytes BITMAPFILEHEADER
     * 40 bytes BITMAPINFOHEADER
     * pixels BGRA 32 bits
     */

    let pixel_size =
        u32::try_from(pixels.len()).map_err(|_| "Screenshot is too large.".to_string())?;

    let pixel_offset = 54u32;

    let file_size = pixel_offset
        .checked_add(pixel_size)
        .ok_or_else(|| "Screenshot file is too large.".to_string())?;

    let mut file = Vec::with_capacity(file_size as usize);

    /*
     * BITMAPFILEHEADER
     */

    file.extend_from_slice(b"BM");

    file.extend_from_slice(&file_size.to_le_bytes());

    file.extend_from_slice(&[0u8; 4]);

    file.extend_from_slice(&pixel_offset.to_le_bytes());

    /*
     * BITMAPINFOHEADER
     */

    file.extend_from_slice(&40u32.to_le_bytes());

    file.extend_from_slice(&width.to_le_bytes());

    /*
     * Hauteur négative =
     * bitmap top-down.
     *
     * Ça évite d'avoir l'image retournée
     * verticalement.
     */
    file.extend_from_slice(&(-height).to_le_bytes());

    file.extend_from_slice(&1u16.to_le_bytes());

    file.extend_from_slice(&32u16.to_le_bytes());

    file.extend_from_slice(&0u32.to_le_bytes());

    file.extend_from_slice(&pixel_size.to_le_bytes());

    file.extend_from_slice(&0i32.to_le_bytes());

    file.extend_from_slice(&0i32.to_le_bytes());

    file.extend_from_slice(&0u32.to_le_bytes());

    file.extend_from_slice(&0u32.to_le_bytes());

    file.extend_from_slice(pixels);

    fs::write(path, file).map_err(|error| format!("Could not write screenshot: {error}"))
}

pub(crate) fn capture_deadlock_test() -> Result<String, String> {
    let hwnd = find_deadlock_window()?;

    let mut rect: RECT = unsafe { zeroed() };

    if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
        return Err("Could not read Deadlock client size.".to_string());
    }

    let width = rect.right - rect.left;

    let height = rect.bottom - rect.top;

    if width <= 0 || height <= 0 {
        return Err(format!("Invalid Deadlock window size: {width}x{height}"));
    }

    println!("[SPLIT] Deadlock capture target: {}x{}", width, height,);

    unsafe {
        /*
         * DC utilisé uniquement pour créer
         * un bitmap compatible.
         */
        let screen_dc = GetDC(null_mut());

        if screen_dc.is_null() {
            return Err("GetDC failed.".to_string());
        }

        let memory_dc = CreateCompatibleDC(screen_dc);

        if memory_dc.is_null() {
            ReleaseDC(null_mut(), screen_dc);

            return Err("CreateCompatibleDC failed.".to_string());
        }

        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);

        if bitmap.is_null() {
            DeleteDC(memory_dc);

            ReleaseDC(null_mut(), screen_dc);

            return Err("CreateCompatibleBitmap failed.".to_string());
        }

        let old_object = SelectObject(memory_dc, bitmap);

        /*
         * 0x00000001 = PW_CLIENTONLY
         * 0x00000002 = PW_RENDERFULLCONTENT
         *
         * On teste volontairement PrintWindow
         * en premier car il peut capturer la
         * fenêtre même si SPLIT est devant.
         */
        let captured = PrintWindow(hwnd, memory_dc, 0x00000001 | 0x00000002);

        if captured == 0 {
            SelectObject(memory_dc, old_object);

            DeleteObject(bitmap);

            DeleteDC(memory_dc);

            ReleaseDC(null_mut(), screen_dc);

            return Err("PrintWindow could not capture Deadlock.".to_string());
        }

        let mut bitmap_info: BITMAPINFO = zeroed();

        bitmap_info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,

            biWidth: width,

            biHeight: -height,

            biPlanes: 1,

            biBitCount: 32,

            biCompression: BI_RGB,

            biSizeImage: 0,

            biXPelsPerMeter: 0,

            biYPelsPerMeter: 0,

            biClrUsed: 0,

            biClrImportant: 0,
        };

        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "Screenshot dimensions overflow.".to_string())?;

        let mut pixels = vec![0u8; pixel_count];

        let scanlines = GetDIBits(
            memory_dc,
            bitmap,
            0,
            height as u32,
            pixels.as_mut_ptr().cast(),
            &mut bitmap_info,
            DIB_RGB_COLORS,
        );

        SelectObject(memory_dc, old_object);

        DeleteObject(bitmap);

        DeleteDC(memory_dc);

        ReleaseDC(null_mut(), screen_dc);

        if scanlines == 0 {
            return Err("GetDIBits failed.".to_string());
        }

        let path = output_path()?;

        write_bmp(&path, width, height, &pixels)?;

        println!(
            "[SPLIT] Deadlock test capture written to {}",
            path.display(),
        );

        Ok(path.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires Deadlock to be running"]
    fn captures_deadlock_window_to_bmp() {
        let path = capture_deadlock_test().expect("Deadlock screenshot test failed");

        println!("Capture written to: {path}");

        assert!(std::path::Path::new(&path,).is_file(),);
    }
}
