//! - [`look at specification(en)`](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/lookAt.md)
//! - [`look at specification(ja)`](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/lookAt.ja.md)

use crate::error::vrm_warn;
use crate::prelude::*;
use crate::system_set::VrmSystemSets;
use bevy::app::{App, Plugin};
use bevy::prelude::*;
use bevy::window::Window;

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
        app.register_type::<LookAt>()
            .register_type::<LookAtProperties>()
            .register_type::<LookAtType>()
            .add_systems(
                PostUpdate,
                track_looking_target
                    .in_set(VrmSystemSets::GazeControl)
                    .after(VrmSystemSets::PropagateAfterConstraints),
            );
    }
}

pub(crate) fn track_looking_target(
    mut commands: Commands,
    vrms: Query<(
        &LookAt,
        &LookAtProperties,
        &HeadBoneEntity,
        &LeftEyeBoneEntity,
        &RightEyeBoneEntity,
    )>,
    transforms: Query<&Transform>,
    global_transforms: Query<&GlobalTransform>,
    rests: Query<(&RestTransform, &RestGlobalTransform)>,
    windows: Query<(Entity, &Window)>,
    cameras: Cameras,
) {
    vrms.iter()
        .for_each(|(look_at, properties, head, left_eye, right_eye)| {
            let Ok(head_gtf) = global_transforms.get(head.0) else {
                return;
            };
            let Ok(head_tf) = transforms.get(head.0) else {
                return;
            };

            let look_at_space = GlobalTransform::default();
            let mut look_at_space_tf = look_at_space.reparented_to(head_gtf);
            look_at_space_tf.translation = Vec3::from(properties.offset_from_head_bone);
            look_at_space_tf.rotation = head_tf.rotation.inverse();
            let look_at_space = head_gtf.mul_transform(look_at_space_tf);

            let (yaw, pitch) = match look_at {
                LookAt::Cursor => {
                    let Some(target_pos) = find_cursor_world_position(&windows, &cameras, head_gtf)
                    else {
                        return;
                    };
                    calc_yaw_pitch(&look_at_space, target_pos)
                }
                LookAt::Target(target_entity) => {
                    let Ok(target_gtf) = global_transforms.get(*target_entity) else {
                        return;
                    };
                    calc_yaw_pitch(&look_at_space, target_gtf.translation())
                }
            };

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
    use crate::tests::{TestResult, test_app};
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
            ))
            .id();
        let eyes = app
            .world_mut()
            .spawn((Transform::default(), GlobalTransform::IDENTITY))
            .id();
        app.world_mut().spawn((
            LookAt::Target(target),
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
}
