//! Icon in the macOS menu bar and the Windows notification area. With it, closing the window hides
//! the app instead of quitting, so the scan stays in memory; the icon brings the window back or
//! quits. On Linux this does nothing.

use std::sync::atomic::{AtomicBool, Ordering};

static QUIT: AtomicBool = AtomicBool::new(false);

/// Mark the next window close as a real quit rather than a hide.
pub fn request_quit() {
    QUIT.store(true, Ordering::Relaxed);
}

pub fn quit_requested() -> bool {
    QUIT.load(Ordering::Relaxed)
}

#[cfg(any(target_os = "macos", test))]
const ICON_SIZE: usize = 36;

/// The app icon's three usage bars as a square RGBA image: black with alpha only, which is what
/// a menu bar template image wants (macOS tints it for light and dark bars).
#[cfg(any(target_os = "macos", test))]
fn bars_icon() -> Vec<u8> {
    const LEFT: f32 = 4.0;
    const RIGHT: f32 = 32.0;
    const RADIUS: f32 = 3.0;
    // Coverage of a pixel by a horizontal pill from LEFT to `right`, soft over one pixel at the edge.
    let pill = |x: f32, y: f32, cy: f32, right: f32| {
        let dx = x - x.clamp(LEFT + RADIUS, right - RADIUS);
        (RADIUS - dx.hypot(y - cy) + 0.5).clamp(0.0, 1.0)
    };
    let mut rgba = vec![0u8; ICON_SIZE * ICON_SIZE * 4];
    for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i % ICON_SIZE) as f32 + 0.5, (i / ICON_SIZE) as f32 + 0.5);
        let mut alpha = 0.0f32;
        for (cy, fill) in [(9.0, 1.0), (18.0, 0.62), (27.0, 0.34)] {
            let filled = pill(x, y, cy, LEFT + (RIGHT - LEFT) * fill);
            let track = pill(x, y, cy, RIGHT) * 0.35;
            alpha = alpha.max(filled).max(track);
        }
        px[3] = (alpha * 255.0).round() as u8;
    }
    rgba
}

#[cfg(any(target_os = "macos", windows))]
mod imp {
    use super::*;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    /// Holds the icon: dropping it removes it from the menu bar or notification area.
    pub struct Tray {
        _icon: TrayIcon,
    }

    /// Call from eframe's creation callback: the event loop runs and this is the main thread.
    pub fn install(cc: &eframe::CreationContext<'_>) -> Option<Tray> {
        if !native::init(cc) {
            return None;
        }
        let menu = Menu::new();
        menu.append_items(&[
            &MenuItem::with_id("show", "Show RustDU", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("quit", "Quit RustDU", true, None),
        ])
        .ok()?;
        let icon = native::icon(
            TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("RustDU")
                // Windows convention: left click opens the app, right click the menu.
                .with_menu_on_left_click(cfg!(target_os = "macos")),
        )?
        .build()
        .ok()?;
        // A hidden window gets no frames, so it is brought back from these handlers rather than
        // from the frame loop; quitting also goes through a frame, to close the same way as File › Quit.
        let ctx = cc.egui_ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id.0 == "quit" {
                request_quit();
            }
            native::show();
            ctx.request_repaint();
        }));
        if cfg!(windows) {
            let ctx = cc.egui_ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    native::show();
                    ctx.request_repaint();
                }
            }));
        }
        Some(Tray { _icon: icon })
    }

    pub use native::hide as hide_app;

    #[cfg(target_os = "macos")]
    mod native {
        use super::super::{ICON_SIZE, bars_icon};
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        use tray_icon::{Icon, TrayIconBuilder};

        pub fn init(_cc: &eframe::CreationContext<'_>) -> bool {
            true
        }

        pub fn icon(builder: TrayIconBuilder) -> Option<TrayIconBuilder> {
            let icon = Icon::from_rgba(bars_icon(), ICON_SIZE as u32, ICON_SIZE as u32).ok()?;
            Some(builder.with_icon_templated(icon))
        }

        /// Hide every window like ⌘H does. The Dock icon and the menu bar icon bring them back.
        pub fn hide() {
            if let Some(mtm) = MainThreadMarker::new() {
                NSApplication::sharedApplication(mtm).hide(None);
            }
        }

        pub fn show() {
            if let Some(mtm) = MainThreadMarker::new() {
                let app = NSApplication::sharedApplication(mtm);
                app.unhide(None);
                // macOS 14 replaced the forceful call with `activate`; the bundle still supports 11.
                if objc2::available!(macos = 14.0) {
                    app.activate();
                } else {
                    #[allow(deprecated)]
                    app.activateIgnoringOtherApps(true);
                }
            }
        }
    }

    #[cfg(windows)]
    mod native {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use std::sync::atomic::{AtomicIsize, Ordering};
        use tray_icon::{Icon, TrayIconBuilder};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow,
        };

        /// The main window, for the tray handlers that run outside the frame loop.
        static WINDOW: AtomicIsize = AtomicIsize::new(0);

        pub fn init(cc: &eframe::CreationContext<'_>) -> bool {
            let Ok(handle) = cc.window_handle() else {
                return false;
            };
            let RawWindowHandle::Win32(h) = handle.as_raw() else {
                return false;
            };
            WINDOW.store(h.hwnd.get(), Ordering::Relaxed);
            true
        }

        pub fn icon(builder: TrayIconBuilder) -> Option<TrayIconBuilder> {
            // The icon build.rs embeds in the exe; winresource gives it id 1.
            Some(builder.with_icon(Icon::from_resource(1, None).ok()?))
        }

        fn window() -> windows_sys::Win32::Foundation::HWND {
            WINDOW.load(Ordering::Relaxed) as _
        }

        pub fn hide() {
            // SAFETY: the handle is the app's own top-level window, alive for the whole run.
            unsafe {
                ShowWindow(window(), SW_HIDE);
            }
        }

        pub fn show() {
            let hwnd = window();
            // SAFETY: as in `hide`.
            unsafe {
                let how = if IsIconic(hwnd) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                };
                ShowWindow(hwnd, how);
                SetForegroundWindow(hwnd);
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    pub struct Tray;

    pub fn install(_cc: &eframe::CreationContext<'_>) -> Option<Tray> {
        None
    }

    pub fn hide_app() {}
}

pub use imp::{Tray, hide_app, install};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_is_three_bars_on_a_clear_background() {
        let rgba = bars_icon();
        assert_eq!(rgba.len(), ICON_SIZE * ICON_SIZE * 4);
        let alpha = |x: usize, y: usize| rgba[(y * ICON_SIZE + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(18, 13), 0, "gap between the bars");
        assert_eq!(alpha(18, 9), 255, "top bar is full");
        assert_eq!(alpha(10, 27), 255, "filled part of the bottom bar");
        assert!((1..255).contains(&alpha(28, 27)), "its track is fainter");
        assert!(rgba.as_chunks::<4>().0.iter().all(|p| p[..3] == [0, 0, 0]));
    }
}
