//! Resolves the mouse cursor into a world-space ray, once per frame.

use crate::prelude::Cameras;
use bevy::math::Vec2;
use bevy::prelude::{Dir3, Entity, Query, Ray3d, ResMut, Resource};
use bevy::window::{RawHandleWrapper, Window};

/// Windows that may contain the cursor, with their native handles (if any).
pub(crate) type CursorWindows<'w, 's> =
    Query<'w, 's, (Entity, &'static Window, Option<&'static RawHandleWrapper>)>;

/// A ray from the camera through the cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CursorRay {
    /// The ray passing through the cursor position.
    pub ray: Ray3d,
    /// The camera's backward direction (the normal of planes facing the camera).
    pub camera_back: Dir3,
}

/// The [`CursorRay`] resolved for the current frame.
#[derive(Resource, Default)]
pub(crate) struct CursorRayThisFrame(pub Option<CursorRay>);

impl CursorRay {
    /// Finds the window containing the cursor and casts a ray through it.
    ///
    /// Windows reporting `cursor_position()` win; only when none does, the platform
    /// fallback is tried, so that a window behind another one is not chosen wrongly.
    pub(crate) fn find(
        windows: &CursorWindows,
        cameras: &Cameras,
    ) -> Option<Self> {
        let (window_entity, cursor_pos) = windows
            .iter()
            .find_map(|(entity, window, _)| Some((entity, window.cursor_position()?)))
            .or_else(|| {
                windows.iter().find_map(|(entity, window, handle)| {
                    Some((entity, fallback_window_cursor_position(window, handle)?))
                })
            })?;
        let (_, camera, _, camera_gtf, _) = cameras.find_camera_from_window(window_entity)?;
        let ray = camera.viewport_to_world(camera_gtf, cursor_pos).ok()?;
        Some(Self {
            ray,
            camera_back: camera_gtf.back(),
        })
    }
}

/// Stores this frame's [`CursorRay`] into [`CursorRayThisFrame`].
pub(crate) fn update_cursor_ray(
    windows: CursorWindows,
    cameras: Cameras,
    mut out: ResMut<CursorRayThisFrame>,
) {
    out.0 = CursorRay::find(&windows, &cameras);
}

/// Cursor position in `window`'s logical coordinates, obtained from the OS
/// when `Window::cursor_position()` is `None` (e.g. `hit_test = false`).
///
/// Always `None` on platforms other than Windows.
#[cfg(target_os = "windows")]
pub(crate) fn fallback_window_cursor_position(
    window: &Window,
    handle: Option<&RawHandleWrapper>,
) -> Option<Vec2> {
    let client = client_cursor_physical(handle?)?;
    client_to_window_logical(client, window.scale_factor(), window.resolution.size())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn fallback_window_cursor_position(
    _window: &Window,
    _handle: Option<&RawHandleWrapper>,
) -> Option<Vec2> {
    None
}

/// Converts a client-area physical position to window logical coordinates,
/// returning `None` when it lies outside the window (edges inclusive).
pub(crate) fn client_to_window_logical(
    client_physical: Vec2,
    scale_factor: f32,
    logical_size: Vec2,
) -> Option<Vec2> {
    let logical = client_physical / scale_factor;
    (logical.x >= 0.0
        && logical.y >= 0.0
        && logical.x <= logical_size.x
        && logical.y <= logical_size.y)
        .then_some(logical)
}

#[cfg(target_os = "windows")]
fn client_cursor_physical(handle: &RawHandleWrapper) -> Option<Vec2> {
    use raw_window_handle::RawWindowHandle;
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let RawWindowHandle::Win32(win32) = handle.get_window_handle() else {
        return None;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut c_void);
    let mut point = POINT::default();
    // SAFETY: `point` is a valid out-pointer, and `hwnd` comes from the live
    // window handle held by `RawHandleWrapper`.
    unsafe {
        GetCursorPos(&mut point).ok()?;
        if !ScreenToClient(hwnd, &mut point).as_bool() {
            return None;
        }
    }
    Some(Vec2::new(point.x as f32, point.y as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_coordinates_are_divided_by_the_scale_factor() {
        let size = Vec2::new(800.0, 600.0);
        assert_eq!(
            client_to_window_logical(Vec2::new(150.0, 150.0), 1.5, size),
            Some(Vec2::new(100.0, 100.0))
        );
        assert_eq!(
            client_to_window_logical(Vec2::new(1200.0, 900.0), 1.5, size),
            Some(Vec2::new(800.0, 600.0))
        );
    }

    #[test]
    fn client_coordinates_outside_the_window_are_rejected() {
        let size = Vec2::new(800.0, 600.0);
        assert_eq!(
            client_to_window_logical(Vec2::new(-1.0, 10.0), 1.5, size),
            None
        );
        assert_eq!(
            client_to_window_logical(Vec2::new(10.0, -1.0), 1.5, size),
            None
        );
        assert_eq!(
            client_to_window_logical(Vec2::new(1201.5, 10.0), 1.5, size),
            None
        );
    }
}
