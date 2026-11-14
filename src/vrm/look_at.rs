//! - [`look at specification(en)`](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/lookAt.md)
//! - [`look at specification(ja)`](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/lookAt.ja.md)

use crate::error::vrm_warn;
use crate::prelude::*;
use crate::system_set::VrmSystemSets;
use bevy::app::{App, Plugin};
use bevy::prelude::*;
use bevy::window::Window;

pub(crate) mod cursor;
pub(crate) mod gaze_target;

use cursor::{CursorRayThisFrame, update_cursor_ray};
use gaze_target::{GazeResolveSet, resolve_gaze_target};

pub use gaze_target::{CursorGazeDepth, GazeTarget};

/// Controls what the VRM model looks at.
/// This component should be inserted into the root entity of the VRM.
///
/// [`LookAt::Cursor`] tracks the mouse cursor across all windows.
/// [`LookAt::Target`] looks at a specified entity.
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
///
/// fn spawn_camera_and_vrm(
///     mut commands: Commands,
///     asset_server: Res<AssetServer>,
/// ) {
///     commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 1.3, 1.0)));
///     commands.spawn((
///         VrmHandle(asset_server.load("model.vrm")),
///         LookAt::Cursor,
///     ));
/// }
/// ```
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub enum LookAt {
    /// Look at the mouse cursor. Automatically finds the window with the cursor
    /// and the `Camera3d` rendering to it.
    Cursor,

    /// Look at a specific target entity.
    Target(Entity),
}

pub(super) struct LookAtPlugin;

impl Plugin for LookAtPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.init_resource::<CursorRayThisFrame>()
            .configure_sets(
                PostUpdate,
                GazeResolveSet
                    .in_set(VrmSystemSets::GazeControl)
                    .after(VrmSystemSets::PropagateAfterConstraints),
            )
            .add_systems(
                PostUpdate,
                (update_cursor_ray, resolve_gaze_target)
                    .chain()
                    .in_set(GazeResolveSet),
            )
            .register_type::<LookAt>()
            .register_type::<CursorGazeDepth>()
            .register_type::<GazeTarget>()
            .register_type::<LookAtProperties>()
            .register_type::<LookAtType>()
            .add_systems(
                PostUpdate,
                track_looking_target
                    .in_set(VrmSystemSets::GazeControl)
                    .after(VrmSystemSets::PropagateAfterConstraints)
                    .after(GazeResolveSet),
            );
    }
}

/// The rotation of the model root's rest pose, or identity when it has none.
///
/// [`RestGlobalTransform`] is relative to the entity that owns the scene, so
/// the rest rotation of the root (for example the rotation of a raw-spawned
/// scene root) has to be re-applied when measuring in world space.
pub(crate) fn root_rest_rotation(
    rests: &Query<(&RestTransform, &RestGlobalTransform)>,
    root: Entity,
) -> Quat {
    rests
        .get(root)
        .map_or(Quat::IDENTITY, |(_, rest_gtf)| rest_gtf.rotation())
}

/// The VRM 1.0 look-at space: it follows the head, but its axes stay aligned
/// with the model's rest frame, so an unrotated model looks along `+Z`.
pub(crate) fn eye_look_at_space(
    head_gtf: &GlobalTransform,
    head_rest: &RestGlobalTransform,
    root_rest_rot: Quat,
    offset: Vec3,
) -> GlobalTransform {
    GlobalTransform::from(Transform {
        translation: head_gtf.transform_point(offset),
        rotation: head_gtf.rotation() * head_rest.rotation().inverse() * root_rest_rot,
        scale: Vec3::ONE,
    })
}

pub(crate) fn track_looking_target(
    mut commands: Commands,
    vrms: Query<
        (
            Entity,
            &GazeTarget,
            &LookAtProperties,
            &HeadBoneEntity,
            &LeftEyeBoneEntity,
            &RightEyeBoneEntity,
        ),
        With<LookAt>,
    >,
    transforms: Query<&Transform>,
    global_transforms: Query<&GlobalTransform>,
    rests: Query<(&RestTransform, &RestGlobalTransform)>,
) {
    vrms.iter()
        .for_each(|(root, gaze, properties, head, left_eye, right_eye)| {
            let Ok(head_gtf) = global_transforms.get(head.0) else {
                return;
            };
            let Ok((_, head_rest)) = rests.get(head.0) else {
                return;
            };
            let look_at_space = eye_look_at_space(
                head_gtf,
                head_rest,
                root_rest_rotation(&rests, root),
                Vec3::from(properties.offset_from_head_bone),
            );
            let (yaw, pitch) = calc_yaw_pitch(&look_at_space, gaze.position());

            match properties.r#type {
                LookAtType::Bone => {
                    apply_bone(
                        &mut commands,
                        &transforms,
                        &rests,
                        left_eye,
                        right_eye,
                        properties,
                        yaw,
                        pitch,
                    );
                }
                LookAtType::Expression => {
                    bevy::utils::once!(vrm_warn!(
                        "Expression look at is not supported yet, skipping gaze control"
                    ));
                }
            }
        });
}

fn apply_bone(
    commands: &mut Commands,
    transforms: &Query<&Transform>,
    rests: &Query<(&RestTransform, &RestGlobalTransform)>,
    left_eye: &LeftEyeBoneEntity,
    right_eye: &RightEyeBoneEntity,
    properties: &LookAtProperties,
    yaw: f32,
    pitch: f32,
) {
    let Ok(left_eye_tf) = transforms.get(left_eye.0) else {
        return;
    };
    let Ok(right_eye_tf) = transforms.get(right_eye.0) else {
        return;
    };
    let Ok((left_eye_rest_tf, left_eye_gtf)) = rests.get(left_eye.0) else {
        return;
    };
    let Ok((right_eye_rest_tf, right_eye_gtf)) = rests.get(right_eye.0) else {
        return;
    };
    let applied_left_eye_tf = apply_left_eye_bone(
        left_eye_tf,
        left_eye_rest_tf,
        left_eye_gtf,
        properties,
        yaw,
        pitch,
    );
    let applied_right_eye_tf = apply_right_eye_bone(
        right_eye_tf,
        right_eye_rest_tf,
        right_eye_gtf,
        properties,
        yaw,
        pitch,
    );
    commands.entity(left_eye.0).insert(applied_left_eye_tf);
    commands.entity(right_eye.0).insert(applied_right_eye_tf);
}

pub(crate) fn find_cursor_world_position(
    windows: &Query<(Entity, &Window)>,
    cameras: &Cameras,
    head_gtf: &GlobalTransform,
) -> Option<Vec3> {
    let (window_entity, cursor_pos) = windows.iter().find_map(|(entity, window)| {
        let cursor = window.cursor_position();

        #[cfg(target_os = "windows")]
        let cursor = {
            let fallback = fallback_cursor_position(window);
            cursor.or(fallback)
        };

        Some((entity, cursor?))
    })?;
    cameras.to_world_by_viewport(window_entity, cursor_pos, head_gtf.translation())
}

/// Fallback cursor position using `WinAPI` `GetCursorPos` for when
/// `Window::cursor_position()` returns `None` (e.g. `hit_test = false`).
#[cfg(target_os = "windows")]
fn fallback_cursor_position(window: &Window) -> Option<Vec2> {
    use bevy::window::WindowPosition;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT::default();
    // SAFETY: GetCursorPos is a safe WinAPI call that writes cursor screen coordinates.
    unsafe { GetCursorPos(&mut point).ok()? };

    let WindowPosition::At(window_pos) = window.position else {
        return None;
    };

    let scale = window.scale_factor();
    let global_logical = Vec2::new(point.x as f32 / scale, point.y as f32 / scale);
    let window_logical = global_logical - window_pos.as_vec2();

    let size = window.resolution.size();
    if window_logical.x >= 0.0
        && window_logical.y >= 0.0
        && window_logical.x <= size.x
        && window_logical.y <= size.y
    {
        Some(window_logical)
    } else {
        None
    }
}

pub(crate) fn calc_yaw_pitch(
    look_at_space: &GlobalTransform,
    target: Vec3,
) -> (f32, f32) {
    let local_target = look_at_space.to_matrix().inverse().transform_point3(target);

    let z = local_target.dot(Vec3::Z);
    let x = local_target.dot(Vec3::X);
    let yaw = (x.atan2(z)).to_degrees();

    let xz = (x * x + z * z).sqrt();
    let y = local_target.dot(Vec3::Y);
    let pitch = (-y.atan2(xz)).to_degrees();

    (yaw, pitch)
}

fn apply_left_eye_bone(
    left_eye: &Transform,
    rest_tf: &RestTransform,
    rest_gtf: &RestGlobalTransform,
    properties: &LookAtProperties,
    yaw_degrees: f32,
    pitch_degrees: f32,
) -> Transform {
    let range_map_horizontal_outer = properties.range_map_horizontal_outer;
    let range_map_horizontal_inner = properties.range_map_horizontal_inner;
    let range_map_vertical_down = properties.range_map_vertical_down;
    let range_map_vertical_up = properties.range_map_vertical_up;
    let yaw = if yaw_degrees > 0.0 {
        yaw_degrees.min(range_map_horizontal_outer.input_max_value)
            / range_map_horizontal_outer.input_max_value
            * range_map_horizontal_outer.output_scale
    } else {
        -(yaw_degrees
            .abs()
            .min(range_map_horizontal_inner.input_max_value)
            / range_map_horizontal_inner.input_max_value
            * range_map_horizontal_inner.output_scale)
    };

    let pitch = if pitch_degrees > 0.0 {
        pitch_degrees.min(range_map_vertical_down.input_max_value)
            / range_map_vertical_down.input_max_value
            * range_map_vertical_down.output_scale
    } else {
        -(pitch_degrees
            .abs()
            .min(range_map_vertical_up.input_max_value)
            / range_map_vertical_up.input_max_value
            * range_map_vertical_up.output_scale)
    };
    left_eye.with_rotation(to_eye_rotation(yaw, pitch, rest_tf, rest_gtf))
}

fn apply_right_eye_bone(
    right_eye: &Transform,
    rest_tf: &RestTransform,
    rest_gtf: &RestGlobalTransform,
    properties: &LookAtProperties,
    yaw_degrees: f32,
    pitch_degrees: f32,
) -> Transform {
    let range_map_horizontal_outer = properties.range_map_horizontal_outer;
    let range_map_horizontal_inner = properties.range_map_horizontal_inner;
    let range_map_vertical_down = properties.range_map_vertical_down;
    let range_map_vertical_up = properties.range_map_vertical_up;

    let yaw = if yaw_degrees > 0.0 {
        yaw_degrees.min(range_map_horizontal_inner.input_max_value)
            / range_map_horizontal_inner.input_max_value
            * range_map_horizontal_inner.output_scale
    } else {
        -(yaw_degrees
            .abs()
            .min(range_map_horizontal_outer.input_max_value)
            / range_map_horizontal_outer.input_max_value
            * range_map_horizontal_outer.output_scale)
    };

    let pitch = if pitch_degrees > 0.0 {
        pitch_degrees.min(range_map_vertical_down.input_max_value)
            / range_map_vertical_down.input_max_value
            * range_map_vertical_down.output_scale
    } else {
        -(pitch_degrees
            .abs()
            .min(range_map_vertical_up.input_max_value)
            / range_map_vertical_up.input_max_value
            * range_map_vertical_up.output_scale)
    };

    right_eye.with_rotation(to_eye_rotation(yaw, pitch, rest_tf, rest_gtf))
}

#[inline]
fn to_eye_rotation(
    yaw: f32,
    pitch: f32,
    rest_tf: &RestTransform,
    rest_gtf: &RestGlobalTransform,
) -> Quat {
    (rest_tf.rotation * rest_gtf.rotation().inverse())
        * Quat::from_euler(EulerRot::YXZ, yaw.to_radians(), pitch.to_radians(), 0.0)
        * rest_gtf.rotation()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::{TestResult, rotation_is, test_app};
    use bevy::ecs::system::RunSystemOnce;

    fn expression_properties() -> LookAtProperties {
        let range = RangeMap {
            input_max_value: 90.0,
            output_scale: 15.0,
        };
        LookAtProperties {
            offset_from_head_bone: [0.0, 0.0, 0.0],
            range_map_horizontal_inner: range,
            range_map_horizontal_outer: range,
            range_map_vertical_down: range,
            range_map_vertical_up: range,
            r#type: LookAtType::Expression,
        }
    }

    #[test]
    fn expression_look_at_leaves_the_eyes_untouched() -> TestResult {
        let mut app = test_app();

        let target = app
            .world_mut()
            .spawn(GlobalTransform::from_xyz(0.0, 1.3, 1.0))
            .id();
        let head = app
            .world_mut()
            .spawn((
                Transform::default(),
                GlobalTransform::from_xyz(0.0, 1.4, 0.0),
                RestTransform(Transform::default()),
                RestGlobalTransform(GlobalTransform::from_xyz(0.0, 1.4, 0.0)),
            ))
            .id();
        let eyes = app
            .world_mut()
            .spawn((Transform::default(), GlobalTransform::IDENTITY))
            .id();
        app.world_mut().spawn((
            LookAt::Target(target),
            GazeTarget::new(Vec3::new(0.0, 1.3, 1.0)),
            expression_properties(),
            HeadBoneEntity(head),
            LeftEyeBoneEntity(eyes),
            RightEyeBoneEntity(eyes),
        ));

        app.world_mut()
            .run_system_once(track_looking_target)
            .expect("Failed to run system");
        app.world_mut().flush();

        assert_eq!(
            *app.world().get::<Transform>(eyes).unwrap(),
            Transform::default()
        );
        success!()
    }

    const EYE_REST_DEGREES: f32 = 21.8;

    struct EyeRig {
        root: Entity,
        left_eye: Entity,
        right_eye: Entity,
        gaze_origin: Vec3,
    }

    fn left_eye_rest() -> Quat {
        Quat::from_rotation_y(EYE_REST_DEGREES.to_radians())
    }

    fn right_eye_rest() -> Quat {
        Quat::from_rotation_y(-EYE_REST_DEGREES.to_radians())
    }

    fn bone_properties() -> LookAtProperties {
        let range = |max, scale| RangeMap {
            input_max_value: max,
            output_scale: scale,
        };
        LookAtProperties {
            offset_from_head_bone: [0.0, 0.06, 0.0],
            range_map_horizontal_inner: range(90.0, 8.0),
            range_map_horizontal_outer: range(90.0, 12.0),
            range_map_vertical_down: range(90.0, 10.0),
            range_map_vertical_up: range(90.0, 10.0),
            r#type: LookAtType::Bone,
        }
    }

    /// Builds a rig by hand because the test app has no transform propagation.
    ///
    /// `root_rot` is the root's current world rotation and `root_rest_rot` the
    /// rotation of its rest pose (`None` means the root has no rest
    /// components). `head_turn` is applied in model space on the left of the
    /// head's current global rotation.
    fn spawn_eye_rig(
        app: &mut App,
        root_rot: Quat,
        root_rest_rot: Option<Quat>,
        neck_rest: Quat,
        head_rest_local: Quat,
        head_turn: Quat,
    ) -> EyeRig {
        let neck_local = Transform::from_xyz(0.0, 1.3, 0.0).with_rotation(neck_rest);
        let head_local = Transform::from_xyz(0.0, 0.07, 0.0).with_rotation(head_rest_local);
        let left_local = Transform::from_xyz(0.016, 0.056, 0.019).with_rotation(left_eye_rest());
        let right_local = Transform::from_xyz(-0.016, 0.056, 0.019).with_rotation(right_eye_rest());

        let root_gtf = GlobalTransform::from(Transform::from_rotation(root_rot));
        let head_gtf = root_gtf.mul_transform(neck_local).mul_transform(head_local);
        let (_, head_rot, head_pos) = head_gtf.to_scale_rotation_translation();
        let head_gtf = GlobalTransform::from(Transform {
            translation: head_pos,
            rotation: head_turn * head_rot,
            scale: Vec3::ONE,
        });

        let rest_root = GlobalTransform::from(Transform::from_rotation(
            root_rest_rot.unwrap_or(Quat::IDENTITY),
        ));
        let rest_head = rest_root
            .mul_transform(neck_local)
            .mul_transform(head_local);

        let gaze_origin = head_gtf.transform_point(Vec3::new(0.0, 0.06, 0.0));

        let mut root = app.world_mut().spawn((
            LookAt::Cursor,
            bone_properties(),
            Transform::from_rotation(root_rot),
            root_gtf,
        ));
        if let Some(rot) = root_rest_rot {
            root.insert((
                RestTransform(Transform::from_rotation(rot)),
                RestGlobalTransform(rest_root),
            ));
        }
        let root = root.id();

        let head = app
            .world_mut()
            .spawn((
                head_local,
                head_gtf,
                RestTransform(head_local),
                RestGlobalTransform(rest_head),
            ))
            .id();
        let mut spawn_eye = |local: Transform| {
            app.world_mut()
                .spawn((
                    Transform::from_rotation(Quat::IDENTITY),
                    GlobalTransform::IDENTITY,
                    RestTransform(local),
                    RestGlobalTransform(rest_head.mul_transform(local)),
                ))
                .id()
        };
        let left_eye = spawn_eye(left_local);
        let right_eye = spawn_eye(right_local);
        app.world_mut().entity_mut(root).insert((
            HeadBoneEntity(head),
            LeftEyeBoneEntity(left_eye),
            RightEyeBoneEntity(right_eye),
        ));
        EyeRig {
            root,
            left_eye,
            right_eye,
            gaze_origin,
        }
    }

    fn run_tracking(app: &mut App) {
        app.world_mut()
            .run_system_once(track_looking_target)
            .expect("Failed to run system");
        app.world_mut().flush();
    }

    fn assert_eyes_at_rest(
        app: &App,
        rig: &EyeRig,
    ) {
        let left = app.world().get::<Transform>(rig.left_eye).unwrap().rotation;
        let right = app
            .world()
            .get::<Transform>(rig.right_eye)
            .unwrap()
            .rotation;
        assert!(
            rotation_is(left, left_eye_rest()),
            "left eye {left:?} is not at rest"
        );
        assert!(
            rotation_is(right, right_eye_rest()),
            "right eye {right:?} is not at rest"
        );
    }

    #[test]
    fn eyes_stay_at_rest_for_a_target_straight_ahead_of_a_tilted_rest_head() {
        let mut app = test_app();
        let rig = spawn_eye_rig(
            &mut app,
            Quat::IDENTITY,
            None,
            Quat::from_rotation_x(6.85_f32.to_radians()),
            Quat::from_rotation_x(-6.23_f32.to_radians()),
            Quat::IDENTITY,
        );
        app.world_mut()
            .entity_mut(rig.root)
            .insert(GazeTarget::new(rig.gaze_origin + Vec3::Z));

        run_tracking(&mut app);

        assert_eyes_at_rest(&app, &rig);
    }

    #[test]
    fn eyes_follow_a_head_turned_by_animation() {
        let mut app = test_app();
        let turn = Quat::from_rotation_y(20.0_f32.to_radians());
        let rig = spawn_eye_rig(
            &mut app,
            Quat::IDENTITY,
            None,
            Quat::IDENTITY,
            Quat::IDENTITY,
            turn,
        );
        app.world_mut()
            .entity_mut(rig.root)
            .insert(GazeTarget::new(rig.gaze_origin + turn * Vec3::Z));

        run_tracking(&mut app);

        assert_eyes_at_rest(&app, &rig);
    }

    #[test]
    fn a_raw_spawn_measures_eye_gaze_in_the_root_frame() {
        let mut app = test_app();
        let o_s = Quat::from_rotation_y(180.0_f32.to_radians());
        let rig = spawn_eye_rig(
            &mut app,
            o_s,
            Some(Quat::from_rotation_y(90.0_f32.to_radians())),
            Quat::IDENTITY,
            Quat::IDENTITY,
            Quat::IDENTITY,
        );
        app.world_mut()
            .entity_mut(rig.root)
            .insert(GazeTarget::new(rig.gaze_origin + o_s * Vec3::Z));

        run_tracking(&mut app);

        assert_eyes_at_rest(&app, &rig);
    }

    #[test]
    fn eyes_do_not_move_without_look_at() {
        let mut app = test_app();
        let rig = spawn_eye_rig(
            &mut app,
            Quat::IDENTITY,
            None,
            Quat::IDENTITY,
            Quat::IDENTITY,
            Quat::IDENTITY,
        );
        app.world_mut()
            .entity_mut(rig.root)
            .insert(GazeTarget::new(rig.gaze_origin + Vec3::X))
            .remove::<LookAt>();

        run_tracking(&mut app);

        for eye in [rig.left_eye, rig.right_eye] {
            let rot = app.world().get::<Transform>(eye).unwrap().rotation;
            assert!(
                rotation_is(rot, Quat::IDENTITY),
                "eye was rewritten: {rot:?}"
            );
        }
    }
}
