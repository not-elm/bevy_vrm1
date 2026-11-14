//! The world-space point the avatar looks at, and how the cursor produces it.

use crate::prelude::*;
use crate::vrm::look_at::cursor::{CursorRay, CursorRayThisFrame, update_cursor_ray};
use bevy::prelude::*;

/// How far in front of the face, toward the camera, `LookAt::Cursor` places the cursor (metres).
/// Larger values make the eyes and body move less.
/// Defaults to `1.0`; non-finite or non-positive values fall back to `1.0`.
///
/// Insert it on the entity that carries `LookAt` (the VRM root). It is only used for
/// `LookAt::Cursor`.
///
/// `BodyTracking` never measures a forward depth below its `reference_depth`, so a smaller
/// `CursorGazeDepth` makes only the eyes more sensitive. To make the body follow the cursor
/// more strongly, lower `BodyTracking::reference_depth` together with `CursorGazeDepth`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct CursorGazeDepth(pub f32);

impl CursorGazeDepth {
    const DEFAULT_DEPTH: f32 = 1.0;
}

impl Default for CursorGazeDepth {
    fn default() -> Self {
        Self(Self::DEFAULT_DEPTH)
    }
}

/// The world-space point the avatar looks at, resolved by `bevy_vrm1` from `LookAt`.
///
/// It is absent until the first frame a target is resolved, keeps the last point on frames
/// that cannot be resolved, and is removed when `LookAt` is removed and on detach.
/// Do not insert it yourself.
#[derive(Component, Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(Component)]
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

    /// The point where the cursor ray meets the camera-facing plane through
    /// `gaze_origin`, moved `depth` metres toward the camera.
    ///
    /// Returns `None` when the ray does not meet that plane.
    pub(crate) fn from_cursor(
        cursor: &CursorRay,
        gaze_origin: Vec3,
        depth: f32,
    ) -> Option<Self> {
        let point = cursor
            .ray
            .plane_intersection_point(gaze_origin, InfinitePlane3d::new(cursor.camera_back))?;
        Some(Self::new(point + *cursor.camera_back * depth))
    }
}

pub(crate) fn effective_cursor_depth(depth: Option<&CursorGazeDepth>) -> f32 {
    match depth {
        Some(CursorGazeDepth(d)) if d.is_finite() && *d > 0.0 => *d,
        _ => CursorGazeDepth::DEFAULT_DEPTH,
    }
}

/// Resolves `LookAt` into the shared [`GazeTarget`] once per frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GazeResolveSet;

pub(super) struct GazeTargetPlugin;

impl Plugin for GazeTargetPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.configure_sets(
            PostUpdate,
            GazeResolveSet
                .in_set(VrmSystemSets::GazeControl)
                .after(VrmSystemSets::PropagateAfterConstraints),
        )
        .add_systems(
            PostUpdate,
            resolve_gaze_target
                .in_set(GazeResolveSet)
                .after(update_cursor_ray),
        )
        .register_type::<CursorGazeDepth>()
        .register_type::<GazeTarget>();
    }
}

/// Writes each avatar's [`GazeTarget`] from its `LookAt`.
/// When the point cannot be resolved this frame, the previous one is kept.
pub(crate) fn resolve_gaze_target(
    mut commands: Commands,
    mut vrms: Query<(
        Entity,
        &LookAt,
        &LookAtProperties,
        &HeadBoneEntity,
        Option<&CursorGazeDepth>,
        Option<&mut GazeTarget>,
    )>,
    cursor: Res<CursorRayThisFrame>,
    stale: Query<Entity, (With<GazeTarget>, Without<LookAt>)>,
    global_transforms: Query<&GlobalTransform>,
) {
    for (vrm, look_at, properties, head, depth, gaze) in &mut vrms {
        let resolved = match look_at {
            LookAt::Cursor => {
                let (Some(cursor), Ok(head_gtf)) =
                    (cursor.0.as_ref(), global_transforms.get(head.0))
                else {
                    continue;
                };
                let origin = head_gtf.transform_point(Vec3::from(properties.offset_from_head_bone));
                GazeTarget::from_cursor(cursor, origin, effective_cursor_depth(depth))
            }
            LookAt::Target(target) => global_transforms
                .get(*target)
                .ok()
                .map(|gtf| GazeTarget::new(gtf.translation())),
        };
        let Some(resolved) = resolved else {
            continue;
        };
        match gaze {
            Some(mut gaze) => {
                gaze.set_if_neq(resolved);
            }
            None => {
                commands.entity(vrm).try_insert(resolved);
            }
        }
    }
    for entity in &stale {
        commands.entity(entity).try_remove::<GazeTarget>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use crate::vrm::look_at::calc_yaw_pitch;
    use crate::vrm::look_at::cursor::{CursorRay, CursorRayThisFrame};
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;

    fn spawn_avatar(
        app: &mut App,
        head_at: Vec3,
        look_at: LookAt,
    ) -> Entity {
        app.init_resource::<CursorRayThisFrame>();
        let head = app
            .world_mut()
            .spawn(GlobalTransform::from_translation(head_at))
            .id();
        app.world_mut()
            .spawn((
                look_at,
                LookAtProperties {
                    offset_from_head_bone: [0.0; 3],
                    ..default()
                },
                HeadBoneEntity(head),
            ))
            .id()
    }

    fn resolve(app: &mut App) {
        app.world_mut()
            .run_system_once(resolve_gaze_target)
            .unwrap();
    }

    fn position_of(
        app: &App,
        vrm: Entity,
    ) -> Option<Vec3> {
        app.world().get::<GazeTarget>(vrm).map(GazeTarget::position)
    }

    /// Asserts that `LookAt::Target` resolves the target entity's world
    /// position into the avatar's `GazeTarget`.
    ///
    /// Case: an app points an avatar at a prop that stands at a fixed place in
    /// the scene.
    #[test]
    fn a_target_entity_becomes_the_gaze_target() {
        let mut app = App::new();
        let target = app
            .world_mut()
            .spawn(GlobalTransform::from_translation(Vec3::new(1.0, 2.0, 3.0)))
            .id();
        let vrm = spawn_avatar(&mut app, Vec3::ZERO, LookAt::Target(target));
        resolve(&mut app);
        assert_eq!(position_of(&app, vrm), Some(Vec3::new(1.0, 2.0, 3.0)));
    }

    /// Asserts that a camera or another `Vrm` can be a `LookAt::Target`, and
    /// that each avatar's `GazeTarget` is the position of its own target.
    ///
    /// Case: an app makes one avatar look at the camera while a second avatar
    /// looks at a third avatar standing to the side.
    #[test]
    fn a_camera_or_another_vrm_can_be_the_target() {
        let mut app = App::new();
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                GlobalTransform::from_translation(Vec3::new(0.0, 1.0, 5.0)),
            ))
            .id();
        let other = app
            .world_mut()
            .spawn((
                Vrm,
                GlobalTransform::from_translation(Vec3::new(4.0, 0.0, 0.0)),
            ))
            .id();
        let a = spawn_avatar(&mut app, Vec3::ZERO, LookAt::Target(camera));
        let b = spawn_avatar(&mut app, Vec3::ZERO, LookAt::Target(other));
        resolve(&mut app);
        assert_eq!(position_of(&app, a), Some(Vec3::new(0.0, 1.0, 5.0)));
        assert_eq!(position_of(&app, b), Some(Vec3::new(4.0, 0.0, 0.0)));
    }

    /// Asserts that `LookAt::Cursor` places each avatar's `GazeTarget` where
    /// the cursor ray crosses a plane in front of that avatar's head, at the
    /// avatar's own `CursorGazeDepth` or at 1.0 when none is set.
    ///
    /// Case: the user moves the cursor over a window that shows two avatars
    /// standing at different distances, and one of them has a custom gaze
    /// depth.
    #[test]
    fn the_cursor_ray_is_placed_in_front_of_each_avatar() {
        let mut app = App::new();
        let near = spawn_avatar(&mut app, Vec3::new(0.0, 1.3, 0.0), LookAt::Cursor);
        let far = spawn_avatar(&mut app, Vec3::new(0.0, 1.3, -2.0), LookAt::Cursor);
        app.world_mut().entity_mut(far).insert(CursorGazeDepth(0.5));
        app.insert_resource(CursorRayThisFrame(Some(front_cursor(0.1, 1.3))));
        resolve(&mut app);
        assert!(
            position_of(&app, near)
                .unwrap()
                .abs_diff_eq(Vec3::new(0.1, 1.3, 1.0), 1e-5)
        );
        assert!(
            position_of(&app, far)
                .unwrap()
                .abs_diff_eq(Vec3::new(0.1, 1.3, -1.5), 1e-5)
        );
    }

    /// Asserts that an avatar keeps its last `GazeTarget` when the target can
    /// no longer be resolved, rather than losing it.
    ///
    /// Case: an app despawns the entity an avatar was looking at, or the user
    /// moves the cursor out of every window.
    #[test]
    fn an_unresolvable_target_keeps_the_last_point() {
        let mut app = App::new();
        let target = app
            .world_mut()
            .spawn(GlobalTransform::from_translation(Vec3::new(1.0, 2.0, 3.0)))
            .id();
        let vrm = spawn_avatar(&mut app, Vec3::ZERO, LookAt::Target(target));
        resolve(&mut app);
        app.world_mut().despawn(target);
        resolve(&mut app);
        assert_eq!(position_of(&app, vrm), Some(Vec3::new(1.0, 2.0, 3.0)));

        let cursor_vrm = spawn_avatar(&mut app, Vec3::new(0.0, 1.3, 0.0), LookAt::Cursor);
        app.insert_resource(CursorRayThisFrame(Some(front_cursor(0.1, 1.3))));
        resolve(&mut app);
        let before = position_of(&app, cursor_vrm).unwrap();
        app.insert_resource(CursorRayThisFrame(None));
        resolve(&mut app);
        assert_eq!(position_of(&app, cursor_vrm), Some(before));
    }

    /// Asserts that removing `LookAt` from an avatar also removes its
    /// `GazeTarget`.
    ///
    /// Case: an app turns look-at off for an avatar that has been looking at an
    /// entity.
    #[test]
    fn removing_look_at_removes_the_gaze_target() {
        let mut app = App::new();
        let target = app
            .world_mut()
            .spawn(GlobalTransform::from_translation(Vec3::ONE))
            .id();
        let vrm = spawn_avatar(&mut app, Vec3::ZERO, LookAt::Target(target));
        resolve(&mut app);
        assert!(position_of(&app, vrm).is_some());
        app.world_mut().entity_mut(vrm).remove::<LookAt>();
        resolve(&mut app);
        assert!(position_of(&app, vrm).is_none());
    }

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

    /// Asserts that the yaw from the head to the cursor's `GazeTarget` equals
    /// the arctangent of the cursor's sideways offset over the gaze depth, for
    /// offsets to either side and at depths 0.5 and 1.0.
    ///
    /// Case: the user holds the cursor slightly to the left or right of the
    /// face of an avatar that faces the camera.
    #[test]
    fn cursor_target_yaw_is_atan_of_offset_over_depth() {
        for depth in [0.5_f32, 1.0] {
            for dx in [0.1_f32, -0.1] {
                let target = GazeTarget::from_cursor(&front_cursor(dx, 0.0), Vec3::ZERO, depth)
                    .unwrap()
                    .position();
                assert!(target.abs_diff_eq(Vec3::new(dx, 0.0, depth), 1e-5));
                let expected = dx.atan2(depth).to_degrees();
                assert!((yaw_from_origin(Vec3::ZERO, target) - expected).abs() < 1e-3);
            }
        }
    }

    /// Asserts that the yaw to the cursor's `GazeTarget` changes in small
    /// steps, without a jump, as the cursor passes through the point directly
    /// in front of the face.
    ///
    /// Case: the user slowly sweeps the cursor across the middle of an avatar's
    /// face.
    #[test]
    fn cursor_target_is_continuous_across_the_face_centre() {
        let yaws: Vec<f32> = (-10..=10)
            .map(|i| {
                let target =
                    GazeTarget::from_cursor(&front_cursor(i as f32 * 0.001, 0.0), Vec3::ZERO, 1.0)
                        .unwrap()
                        .position();
                yaw_from_origin(Vec3::ZERO, target)
            })
            .collect();
        assert!(
            yaws.windows(2).all(|w| (w[1] - w[0]).abs() < 0.2),
            "{yaws:?}"
        );
    }

    /// Asserts that, for a camera turned by 30 degrees about the vertical axis,
    /// the cursor's `GazeTarget` lies at the gaze depth along the camera's
    /// viewing direction and its yaw changes strictly in one direction, without
    /// jumps, as the cursor moves sideways.
    ///
    /// Case: an app views an avatar through a camera that is turned to the
    /// side, and the user sweeps the cursor across the avatar's face.
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
                let target = GazeTarget::from_cursor(&cursor, Vec3::ZERO, 1.0)
                    .unwrap()
                    .position();
                assert!((target.dot(*back) - 1.0).abs() < 1e-4);
                yaw_from_origin(Vec3::ZERO, target)
            })
            .collect();
        assert!(
            yaws.windows(2).all(|w| w[1] > w[0] && w[1] - w[0] < 1.0),
            "{yaws:?}"
        );
    }

    /// Asserts that a cursor ray that never reaches the plane in front of the
    /// head yields no `GazeTarget`.
    ///
    /// Case: the camera is oriented so that the plane in front of the avatar
    /// lies behind the cursor ray.
    #[test]
    fn a_ray_that_misses_the_plane_yields_no_target() {
        let away = CursorRay {
            ray: Ray3d::new(Vec3::new(0.0, 0.0, 20.0), Dir3::Z),
            camera_back: Dir3::Z,
        };
        assert_eq!(GazeTarget::from_cursor(&away, Vec3::ZERO, 1.0), None);
    }

    /// Asserts that a missing, non-finite, or non-positive cursor depth falls
    /// back to 1.0 rather than being clamped, while a valid depth is used as
    /// set.
    ///
    /// Case: an app sets `CursorGazeDepth(0.0)`, or loads a NaN from a broken
    /// settings file.
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
