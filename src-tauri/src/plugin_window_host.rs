//! The frame of a plug-in's window on Windows: a bar of ours along the top
//! (the plug-in's name and a preset dropdown, drawn by the app's own page)
//! and the plug-in's interface in an area of its own below it.
//!
//! A plug-in's editor attaches to whatever window it is handed and fills it.
//! Handed the Tauri window itself, it covered the whole window and there was
//! nowhere to put anything of ours. Here it is handed a plain child window
//! that starts below the bar, and the window is sized to the bar plus the
//! size the editor asks for.

/// Height of the bar in CSS pixels. The page draws its bar this tall.
pub const HEADER_CSS_PX: f64 = 36.0;

#[cfg(windows)]
pub struct PluginArea {
    child: windows_sys::Win32::Foundation::HWND,
    header: i32,
}

/// Make the plug-in's area inside `window`, below the bar, and return the
/// handle the editor attaches to.
#[cfg(windows)]
pub fn make_area(
    window: &tauri::WebviewWindow,
) -> Result<(raw_window_handle::RawWindowHandle, PluginArea), String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle, Win32WindowHandle};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
    };

    let handle = window
        .window_handle()
        .map_err(|e| format!("window handle unavailable: {e}"))?;
    let RawWindowHandle::Win32(parent) = handle.as_raw() else {
        return Err("not a Windows window".into());
    };
    let parent = parent.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    let scale = window.scale_factor().unwrap_or(1.0);
    let header = (HEADER_CSS_PX * scale).round() as i32;
    let size = window
        .inner_size()
        .map_err(|e| format!("window size unavailable: {e}"))?;
    let class: Vec<u16> = "STATIC".encode_utf16().chain(std::iter::once(0)).collect();
    // Created after the window's web view, so it sits above it; clipped so
    // neither draws over the other.
    let child = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
            0,
            header,
            size.width as i32,
            (size.height as i32 - header).max(1),
            parent,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    let id = std::num::NonZeroIsize::new(child as isize)
        .ok_or_else(|| "could not make the plug-in's area".to_string())?;
    Ok((
        RawWindowHandle::Win32(Win32WindowHandle::new(id)),
        PluginArea { child, header },
    ))
}

/// Destroy an area made by [`make_area`], on the thread that made it, once its plug-in's GUI
/// is closed: the window it was in is kept for the next plug-in (plugin_window_pool).
#[cfg(windows)]
pub fn destroy_area(area: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyWindow, IsWindow};
    let hwnd = area as windows_sys::Win32::Foundation::HWND;
    // SAFETY: an area of ours; one already gone with its window fails IsWindow.
    unsafe {
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
    }
}

#[cfg(windows)]
impl PluginArea {
    /// The area's window handle, for [`destroy_area`].
    pub fn handle(&self) -> isize {
        self.child as isize
    }

    /// Size the area to the editor and the window to the bar plus the area.
    pub fn fit(&self, window: &tauri::WebviewWindow, width: u32, height: u32) {
        fit_area(window, self.child as isize, self.header, width, height);
    }
}

/// The bar's height in physical pixels for `window`'s screen.
#[cfg(windows)]
pub fn header_px(window: &tauri::WebviewWindow) -> i32 {
    (HEADER_CSS_PX * window.scale_factor().unwrap_or(1.0)).round() as i32
}

#[cfg(windows)]
thread_local! {
    /// Set while the host sizes a window itself, so the resize that causes
    /// is not handed back to the plug-in as if the person had dragged.
    static FITTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the host is sizing a window right now (see `FITTING`).
#[cfg(windows)]
pub fn fitting() -> bool {
    FITTING.with(|f| f.get())
}

/// Size the area `area` to `width` by `height` below a bar `header` tall,
/// and the window around them.
#[cfg(windows)]
pub fn fit_area(window: &tauri::WebviewWindow, area: isize, header: i32, width: u32, height: u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOP, SWP_NOACTIVATE};
    FITTING.with(|f| f.set(true));
    unsafe {
        SetWindowPos(
            area as windows_sys::Win32::Foundation::HWND,
            HWND_TOP,
            0,
            header,
            width as i32,
            height as i32,
            SWP_NOACTIVATE,
        );
    }
    let _ = window.set_size(tauri::PhysicalSize::new(width, height + header as u32));
    FITTING.with(|f| f.set(false));
}

/// Move the area only, for a window the person sized: the window keeps
/// the size they gave it.
#[cfg(windows)]
pub fn place_area(area: isize, header: i32, width: u32, height: u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPos, HWND_TOP, SWP_NOACTIVATE};
    unsafe {
        SetWindowPos(
            area as windows_sys::Win32::Foundation::HWND,
            HWND_TOP,
            0,
            header,
            width as i32,
            height as i32,
            SWP_NOACTIVATE,
        );
    }
}
