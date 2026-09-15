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
            DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
            BI_RGB, DIB_RGB_COLORS, SRCCOPY,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetClientRect, GetWindowThreadProcessId, IsWindowVisible,
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

fn write_thumbnail(
    path: &PathBuf,
    width: i32,
    height: i32,
    pixels: &mut [u8],
) -> Result<(), String> {
    use image::{
        codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage, ExtendedColorType, RgbaImage,
    };

    /*
     * GetDIBits nous donne du BGRA.
     *
     * image attend du RGBA :
     * on inverse donc B et R.
     */
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);

        /*
         * Alpha opaque.
         */
        pixel[3] = 255;
    }

    let width = u32::try_from(width).map_err(|_| "Invalid screenshot width.".to_string())?;

    let height = u32::try_from(height).map_err(|_| "Invalid screenshot height.".to_string())?;

    let image = RgbaImage::from_raw(width, height, pixels.to_vec())
        .ok_or_else(|| "Could not build screenshot image.".to_string())?;

    /*
     * Taille finale des miniatures SPLIT.
     *
     * resize_to_fill conserve le ratio visuel
     * et crop légèrement si nécessaire.
     */
    let thumbnail = DynamicImage::ImageRgba8(image)
        .resize_to_fill(640, 360, FilterType::Triangle)
        .to_rgb8();

    let file =
        fs::File::create(path).map_err(|error| format!("Could not create screenshot: {error}"))?;

    let mut encoder = JpegEncoder::new_with_quality(file, 82);

    encoder
        .encode(
            thumbnail.as_raw(),
            thumbnail.width(),
            thumbnail.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|error| format!("Could not encode screenshot: {error}"))?;

    Ok(())
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

        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);

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

        let captured = BitBlt(
            memory_dc, 0, 0, width, height, screen_dc, origin.x, origin.y, SRCCOPY,
        );

        if captured == 0 {
            SelectObject(memory_dc, old_object);

            DeleteObject(bitmap);
            DeleteDC(memory_dc);

            ReleaseDC(null_mut(), screen_dc);

            return Err("BitBlt could not capture Deadlock.".to_string());
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

        write_thumbnail(&path, width, height, &mut pixels)?;

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
