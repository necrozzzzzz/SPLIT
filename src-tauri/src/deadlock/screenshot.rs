use std::{
    fs,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::null_mut,
};

use windows_sys::{
    core::BOOL,
    Win32::{
        Foundation::{HWND, LPARAM, POINT, RECT},
        Graphics::Gdi::{
            BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC,
            DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject, SetStretchBltMode, StretchBlt,
            BITMAPINFO, BITMAPINFOHEADER, BI_RGB, COLORONCOLOR, DIB_RGB_COLORS, SRCCOPY,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetClientRect, GetWindowThreadProcessId, IsWindowVisible,
        },
    },
};

const THUMB_WIDTH: i32 = 640;
const THUMB_HEIGHT: i32 = 360;

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
    use std::time::{SystemTime, UNIX_EPOCH};

    let appdata =
        std::env::var_os("APPDATA").ok_or_else(|| "APPDATA is unavailable.".to_string())?;

    let directory = PathBuf::from(appdata).join("SPLIT").join("screenshots");

    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create screenshot directory: {error}"))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?
        .as_millis();

    Ok(directory.join(format!("capture-{timestamp}.jpg")))
}

fn full_output_path() -> Result<PathBuf, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let appdata =
        std::env::var_os("APPDATA").ok_or_else(|| "APPDATA is unavailable.".to_string())?;

    let directory = PathBuf::from(appdata).join("SPLIT").join("screenshots");

    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create screenshot directory: {error}"))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?
        .as_millis();

    Ok(directory.join(format!("capture-full-{timestamp}.jpg")))
}

fn write_full_res_jpeg(
    path: &PathBuf,
    width: i32,
    height: i32,
    pixels: &[u8],
) -> Result<(), String> {
    use image::{codecs::jpeg::JpegEncoder, ExtendedColorType};

    let pixel_count = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| "Full screenshot dimensions overflow.".to_string())?;

    let mut rgb = Vec::with_capacity(pixel_count);

    /*
     * GetDIBits fournit du BGRA.
     * JPEG attend du RGB.
     */
    for pixel in pixels.chunks_exact(4) {
        rgb.push(pixel[2]);
        rgb.push(pixel[1]);
        rgb.push(pixel[0]);
    }

    let file = fs::File::create(path)
        .map_err(|error| format!("Could not create full screenshot: {error}"))?;

    let mut encoder = JpegEncoder::new_with_quality(file, 90);

    encoder
        .encode(&rgb, width as u32, height as u32, ExtendedColorType::Rgb8)
        .map_err(|error| format!("Could not encode full screenshot: {error}"))?;

    Ok(())
}

fn write_thumbnail(path: &PathBuf, pixels: &[u8]) -> Result<(), String> {
    use image::{codecs::jpeg::JpegEncoder, ExtendedColorType};

    /*
     * GetDIBits fournit du BGRA.
     *
     * Le JPEG attend du RGB.
     *
     * L'image est déjà en 640x360 grâce
     * à StretchBlt : aucun resize CPU ici.
     */
    let mut rgb = Vec::with_capacity(
        usize::try_from(THUMB_WIDTH * THUMB_HEIGHT * 3)
            .map_err(|_| "Invalid thumbnail size.".to_string())?,
    );

    for pixel in pixels.chunks_exact(4) {
        rgb.push(pixel[2]);
        rgb.push(pixel[1]);
        rgb.push(pixel[0]);
    }

    let file =
        fs::File::create(path).map_err(|error| format!("Could not create screenshot: {error}"))?;

    let mut encoder = JpegEncoder::new_with_quality(file, 82);

    encoder
        .encode(
            &rgb,
            THUMB_WIDTH as u32,
            THUMB_HEIGHT as u32,
            ExtendedColorType::Rgb8,
        )
        .map_err(|error| format!("Could not encode screenshot: {error}"))?;

    Ok(())
}

pub(crate) fn capture_deadlock_full_res_async() -> Result<String, String> {
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

    let pixels = unsafe {
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

        let mut origin = POINT { x: 0, y: 0 };

        if ClientToScreen(hwnd, &mut origin) == 0 {
            SelectObject(memory_dc, old_object);

            DeleteObject(bitmap);
            DeleteDC(memory_dc);

            ReleaseDC(null_mut(), screen_dc);

            return Err("ClientToScreen failed.".to_string());
        }

        /*
         * Copie brute du client Deadlock
         * dans sa résolution native.
         *
         * Aucun encodage JPEG ici.
         */
        let captured = BitBlt(
            memory_dc, 0, 0, width, height, screen_dc, origin.x, origin.y, SRCCOPY,
        );

        if captured == 0 {
            SelectObject(memory_dc, old_object);

            DeleteObject(bitmap);
            DeleteDC(memory_dc);

            ReleaseDC(null_mut(), screen_dc);

            return Err("BitBlt could not capture full Deadlock screenshot.".to_string());
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
            .ok_or_else(|| "Full screenshot dimensions overflow.".to_string())?;

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
            return Err("GetDIBits failed for full screenshot.".to_string());
        }

        pixels
    };

    /*
     * Le chemin est connu immédiatement,
     * mais l'encodage du gros JPEG est lancé
     * sur un thread séparé.
     *
     * Le Save ne doit donc pas attendre
     * plusieurs secondes.
     */
    let path = full_output_path()?;

    let background_path = path.clone();

    std::thread::Builder::new()
        .name("split-full-screenshot".to_string())
        .spawn(
            move || match write_full_res_jpeg(&background_path, width, height, &pixels) {
                Ok(()) => {
                    println!(
                        "[SPLIT] Full screenshot written -> {} ({}x{})",
                        background_path.display(),
                        width,
                        height,
                    );
                }

                Err(error) => {
                    eprintln!("[SPLIT] Full screenshot encode failed: {error}");
                }
            },
        )
        .map_err(|error| format!("Could not start full screenshot encoder: {error}"))?;

    Ok(path.to_string_lossy().into_owned())
}

pub(crate) fn capture_deadlock_thumbnail() -> Result<String, String> {
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

        let bitmap = CreateCompatibleBitmap(screen_dc, THUMB_WIDTH, THUMB_HEIGHT);

        if bitmap.is_null() {
            DeleteDC(memory_dc);

            ReleaseDC(null_mut(), screen_dc);

            return Err("CreateCompatibleBitmap failed.".to_string());
        }

        let old_object = SelectObject(memory_dc, bitmap);

        /*
         * Au moment d'un Save, Deadlock est déjà
         * au premier plan.
         *
         * On copie donc directement les pixels
         * visibles de son client depuis l'écran.
         *
         * BitBlt est beaucoup plus léger que
         * PrintWindow sur une fenêtre de jeu.
         */
        let mut origin = POINT { x: 0, y: 0 };

        if ClientToScreen(hwnd, &mut origin) == 0 {
            SelectObject(memory_dc, old_object);

            DeleteObject(bitmap);
            DeleteDC(memory_dc);

            ReleaseDC(null_mut(), screen_dc);

            return Err("ClientToScreen failed.".to_string());
        }

        /*
         * On croppe la source au ratio 16:9,
         * puis Windows réduit directement
         * l'image vers 640x360.
         */
        let (source_x, source_y, source_width, source_height) =
            if i64::from(width) * 9 > i64::from(height) * 16 {
                let source_width = height * 16 / 9;

                ((width - source_width) / 2, 0, source_width, height)
            } else if i64::from(width) * 9 < i64::from(height) * 16 {
                let source_height = width * 9 / 16;

                (0, (height - source_height) / 2, width, source_height)
            } else {
                (0, 0, width, height)
            };

        let _ = SetStretchBltMode(memory_dc, COLORONCOLOR);

        let captured = StretchBlt(
            memory_dc,
            0,
            0,
            THUMB_WIDTH,
            THUMB_HEIGHT,
            screen_dc,
            origin.x + source_x,
            origin.y + source_y,
            source_width,
            source_height,
            SRCCOPY,
        );

        if captured == 0 {
            SelectObject(memory_dc, old_object);

            DeleteObject(bitmap);
            DeleteDC(memory_dc);

            ReleaseDC(null_mut(), screen_dc);

            return Err("StretchBlt could not capture Deadlock.".to_string());
        }

        let mut bitmap_info: BITMAPINFO = zeroed();

        bitmap_info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,

            biWidth: THUMB_WIDTH,

            biHeight: -THUMB_HEIGHT,

            biPlanes: 1,

            biBitCount: 32,

            biCompression: BI_RGB,

            biSizeImage: 0,

            biXPelsPerMeter: 0,

            biYPelsPerMeter: 0,

            biClrUsed: 0,

            biClrImportant: 0,
        };

        let pixel_count = usize::try_from(THUMB_WIDTH)
            .ok()
            .and_then(|width| {
                usize::try_from(THUMB_HEIGHT)
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
            THUMB_HEIGHT as u32,
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

        write_thumbnail(&path, &pixels)?;

        println!("[SPLIT] Deadlock thumbnail written to {}", path.display(),);

        Ok(path.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires Deadlock to be running"]
    fn captures_deadlock_window_to_jpeg() {
        let path = capture_deadlock_thumbnail().expect("Deadlock screenshot test failed");

        println!("Capture written to: {path}");

        assert!(std::path::Path::new(&path,).is_file(),);
    }
}
