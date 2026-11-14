//! The world-space point the avatar looks at, and how the cursor produces it.

use crate::vrm::look_at::cursor::CursorRay;
use bevy::prelude::*;

/// How far in front of the face, toward the camera, `LookAt::Cursor` places the cursor (metres).
/// Larger values make the eyes and body move less.
/// Defaults to `1.0`; non-finite or non-positive values fall back to `1.0`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct CursorGazeDepth(pub f32);

impl Default for CursorGazeDepth {
    fn default() -> Self {
        Self(DEFAULT_CURSOR_DEPTH)
    }
}

const DEFAULT_CURSOR_DEPTH: f32 = 1.0;

/// The world-space point the avatar looks at this frame, resolved by `bevy_vrm1` from `LookAt`.
/// Do not insert it yourself.
#[derive(Component, Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct GazeTarget {
    position: Vec3,
}

impl GazeTarget {
    /// Returns the world-space position being looked at.
    #[inline]
    pub fn position(&self) -> Vec3 {
        self.position
    }

    #[inline]
    pub(crate) fn new(position: Vec3) -> Self {
        Self { position }
    }
}

pub(crate) fn effective_cursor_depth(depth: Option<&CursorGazeDepth>) -> f32 {
    match depth {
        Some(CursorGazeDepth(d)) if d.is_finite() && *d > 0.0 => *d,
        _ => DEFAULT_CURSOR_DEPTH,
    }
}

/// Intersects the cursor ray with the camera-facing plane through `gaze_origin`,
/// then pulls the point `depth` metres toward the camera.
pub(crate) fn cursor_gaze_target(
    cursor: &CursorRay,
    gaze_origin: Vec3,
    depth: f32,
) -> Option<Vec3> {
    let point = cursor
        .ray
        .plane_intersection_point(gaze_origin, InfinitePlane3d::new(cursor.camera_back))?;
    Some(point + *cursor.camera_back * depth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vrm::look_at::calc_yaw_pitch;
    use crate::vrm::look_at::cursor::CursorRay;
    use bevy::prelude::*;

    fn front_cursor(
        x: f32,
        y: f32,
    ) -> CursorRay {
        CursorRay {
            ray: Ray3d::new(Vec3::new(x, y, 20.0), Dir3::NEG_Z),
            camera_back: Dir3::Z,
        }
    }

    fn yaw_from_origin(
        origin: Vec3,
        target: Vec3,
    ) -> f32 {
        calc_yaw_pitch(&GlobalTransform::from_translation(origin), target).0
    }

    #[test]
    fn cursor_target_yaw_is_atan_of_offset_over_depth() {
        for depth in [0.5_f32, 1.0] {
            for dx in [0.1_f32, -0.1] {
                let target = cursor_gaze_target(&front_cursor(dx, 0.0), Vec3::ZERO, depth).unwrap();
                assert!(target.abs_diff_eq(Vec3::new(dx, 0.0, depth), 1e-5));
                let expected = dx.atan2(depth).to_degrees();
                assert!((yaw_from_origin(Vec3::ZERO, target) - expected).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn cursor_target_is_continuous_across_the_face_centre() {
        let yaws: Vec<f32> = (-10..=10)
            .map(|i| {
                let target =
                    cursor_gaze_target(&front_cursor(i as f32 * 0.001, 0.0), Vec3::ZERO, 1.0)
                        .unwrap();
                yaw_from_origin(Vec3::ZERO, target)
            })
            .collect();
        assert!(
            yaws.windows(2).all(|w| (w[1] - w[0]).abs() < 0.2),
            "{yaws:?}"
        );
    }

    #[test]
    fn cursor_target_is_monotonic_for_a_rotated_camera() {
        let back = Dir3::new(Quat::from_rotation_y(30_f32.to_radians()) * Vec3::Z).unwrap();
        let right = Quat::from_rotation_y(30_f32.to_radians()) * Vec3::X;
        let yaws: Vec<f32> = (-10..=10)
            .map(|i| {
                let origin = back * 20.0 + right * (i as f32 * 0.01);
                let cursor = CursorRay {
                    ray: Ray3d::new(origin, -back),
                    camera_back: back,
                };
                let target = cursor_gaze_target(&cursor, Vec3::ZERO, 1.0).unwrap();
                assert!((target.dot(*back) - 1.0).abs() < 1e-4);
                yaw_from_origin(Vec3::ZERO, target)
            })
            .collect();
        assert!(
            yaws.windows(2).all(|w| w[1] > w[0] && w[1] - w[0] < 1.0),
            "{yaws:?}"
        );
    }

    #[test]
    fn a_ray_that_misses_the_plane_yields_no_target() {
        let away = CursorRay {
            ray: Ray3d::new(Vec3::new(0.0, 0.0, 20.0), Dir3::Z),
            camera_back: Dir3::Z,
        };
        assert_eq!(cursor_gaze_target(&away, Vec3::ZERO, 1.0), None);
    }

    #[test]
    fn invalid_cursor_depths_fall_back_to_one() {
        assert_eq!(effective_cursor_depth(None), 1.0);
        for bad in [0.0, -0.5, f32::NAN, f32::INFINITY] {
            assert_eq!(effective_cursor_depth(Some(&CursorGazeDepth(bad))), 1.0);
        }
        assert_eq!(effective_cursor_depth(Some(&CursorGazeDepth(0.7))), 0.7);
        assert_eq!(CursorGazeDepth::default().0, 1.0);
    }
}
