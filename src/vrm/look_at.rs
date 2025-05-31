//! [`VRMC_vrm-1.0/lookAt.md`](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/lookAt.md)

use crate::vrm::gltf::extensions::vrmc_vrm::{LookAtProperties, LookAtType};
use crate::vrm::{Head, LeftEye, RightEye};
use bevy::app::{App, Plugin};
use bevy::prelude::*;
use bevy::render::camera::RenderTarget;
use bevy::window::{PrimaryWindow, WindowRef};
#[cfg(feature = "reflect")]
use serde::{Deserialize, Serialize};

/// Holds the entity of looking the target entity.
/// This component should be inserted into the root entity of the VRM.
///
/// [`LookAt::Cursor`] is used to look at the mouse cursor in the window.
/// [`LookAt::Target`] is used to look at the specified entity.
///
/// ```no_run
/// use bevy::prelude::*;
///
/// fn spawn_camera_and_vrm(
///     mut commands: Commands,
///     asset_server: Res<AssetServer>,
/// ) {
///     let camera = commands.spawn(Camera3d::default()).id();
///     commands.spawn((
///         VrmHandle(asset_server.load("model.vrm")),
///         LookAt::Cursor {
///             camera,
///         },
///     ));
/// }
/// ```
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
pub enum LookAt {
    /// Look at the window cursor.
    /// The camera entity that is specified as the render target of the window must be passed.
    Cursor { camera: Entity },

    /// Specify the entity of the target.
    Target(Entity),
}

pub struct LookAtPlugin;

impl Plugin for LookAtPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(Update, (spawn_look_at_space, track_looking_target));

        #[cfg(feature = "reflect")]
        app.register_type::<LookAtSpace>();
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
struct LookAtSpace(Entity);

/// See [`lookAt`](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/lookAt.md) for more details.
fn spawn_look_at_space(
    par_commands: ParallelCommands,
    vrms: Query<(Entity, &Head, &LookAtProperties), Added<Head>>,
    transforms: Query<&Transform>,
) {
    vrms.par_iter().for_each(|(entity, head, properties)| {
        let offset = Vec3::from_array(properties.offset_from_head_bone);
        let Ok(head_tf) = transforms.get(head.0) else {
            return;
        };
        let transform =
            Transform::from_translation(offset).with_rotation(head_tf.rotation.inverse());
        par_commands.command_scope(move |mut commands: Commands| {
            let look_at_space = commands.spawn(transform).id();
            commands.entity(head.0).add_child(look_at_space);
            commands.entity(entity).insert(LookAtSpace(look_at_space));
        });
    });
}

fn track_looking_target(
    par_commands: ParallelCommands,
    vrms: Query<(
        &LookAt,
        &LookAtProperties,
        &Head,
        &LookAtSpace,
        &LeftEye,
        &RightEye,
    )>,
    cameras: Query<&Camera>,
    transforms: Query<&Transform>,
    global_transforms: Query<&GlobalTransform>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    windows: Query<&Window, Without<PrimaryWindow>>,
) {
    vrms.par_iter().for_each(
        |(look_at, properties, head, look_at_space, left_eye, right_eye)| {
            let Ok(look_at_space_tf) = transforms.get(look_at_space.0) else {
                return;
            };
            let Ok(head_gtf) = global_transforms.get(head.0) else {
                return;
            };
            let Ok(head_tf) = transforms.get(head.0) else {
                return;
            };
            let mut look_at_space_tf = *look_at_space_tf;
            look_at_space_tf.translation = Vec3::from(properties.offset_from_head_bone);
            look_at_space_tf.rotation = head_tf.rotation.inverse();
            let look_at_space = head_gtf.mul_transform(look_at_space_tf);
            let Some(target) = calc_target_position(
                look_at,
                head.0,
                &transforms,
                &global_transforms,
                &cameras,
                &primary_window,
                &windows,
            ) else {
                return;
            };
            let (yaw, pitch) = calc_yaw_pitch(&look_at_space, target);
            match properties.r#type {
                LookAtType::Bone => {
                    apply_bone(
                        &par_commands,
                        &transforms,
                        left_eye,
                        right_eye,
                        properties,
                        yaw,
                        pitch,
                    );
                }
                LookAtType::Expression => {
                    todo!("Expression look at is not supported yet");
                }
            }
        },
    );
}

fn calc_target_position(
    look_at: &LookAt,
    vrm_entity: Entity,
    transforms: &Query<&Transform>,
    global_transforms: &Query<&GlobalTransform>,
    cameras: &Query<&Camera>,
    primary_window: &Query<&Window, With<PrimaryWindow>>,
    windows: &Query<&Window, Without<PrimaryWindow>>,
) -> Option<Vec3> {
    match look_at {
        LookAt::Cursor { camera } => calc_lookt_at_cursor_position(
            *camera,
            vrm_entity,
            global_transforms,
            cameras,
            primary_window,
            windows,
        ),
        LookAt::Target(target_entity) => transforms.get(*target_entity).map(|t| t.translation).ok(),
    }
}

fn apply_bone(
    par_commands: &ParallelCommands,
    transforms: &Query<&Transform>,
    left_eye: &LeftEye,
    right_eye: &RightEye,
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
    let applied_left_eye_tf = apply_left_eye_bone(left_eye_tf, properties, yaw, pitch);
    let applied_right_eye_tf = apply_right_eye_bone(right_eye_tf, properties, yaw, pitch);
    // Circles have 32 line-segments by default.
    // You may want to increase this for larger circles.

    par_commands.command_scope(move |mut commands: Commands| {
        commands.entity(left_eye.0).insert(applied_left_eye_tf);
        commands.entity(right_eye.0).insert(applied_right_eye_tf);
    });
}

fn calc_lookt_at_cursor_position(
    camera_entity: Entity,
    vrm_entity: Entity,
    global_transforms: &Query<&GlobalTransform>,
    cameras: &Query<&Camera>,
    primary_window: &Query<&Window, With<PrimaryWindow>>,
    windows: &Query<&Window, Without<PrimaryWindow>>,
) -> Option<Vec3> {
    let camera = cameras.get(camera_entity).ok()?;
    let camera_gtf = global_transforms.get(camera_entity).ok()?;
    let head_gtf = global_transforms.get(vrm_entity).ok()?;
    let RenderTarget::Window(window_ref) = camera.target else {
        return None;
    };
    let window = match window_ref {
        WindowRef::Primary => primary_window.single().ok()?,
        WindowRef::Entity(window_entity) => windows.get(window_entity).ok()?,
    };
    let cursor = window.cursor_position()?;
    let ray = camera.viewport_to_world(camera_gtf, cursor).ok()?;
    let plane_origin = head_gtf.translation() + head_gtf.back().as_vec3();
    let plane_up = InfinitePlane3d::new(head_gtf.forward());
    let distance = ray.intersect_plane(plane_origin, plane_up)?;
    println!("applied_left_eye_tf: {:?}", ray.get_point(distance));
    Some(ray.get_point(distance))
}

fn calc_yaw_pitch(
    look_at_space: &GlobalTransform,
    target: Vec3,
) -> (f32, f32) {
    let local_target = look_at_space
        .compute_matrix()
        .inverse()
        .transform_point3(target);

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
    properties: &LookAtProperties,
    yaw_degress: f32,
    pitch_degress: f32,
) -> Transform {
    let range_map_horizontal_outer = properties.range_map_horizontal_outer;
    let range_map_horizontal_inner = properties.range_map_horizontal_inner;
    let range_map_vertical_down = properties.range_map_vertical_down;
    let range_map_vertical_up = properties.range_map_vertical_up;
    let yaw = if yaw_degress > 0.0 {
        yaw_degress.min(range_map_horizontal_outer.input_max_value)
            / range_map_horizontal_outer.input_max_value
            * range_map_horizontal_outer.output_scale
    } else {
        -(yaw_degress
            .abs()
            .min(range_map_horizontal_inner.input_max_value)
            / range_map_horizontal_inner.input_max_value
            * range_map_horizontal_inner.output_scale)
    };

    let pitch = if pitch_degress > 0.0 {
        pitch_degress.min(range_map_vertical_down.input_max_value)
            / range_map_vertical_down.input_max_value
            * range_map_vertical_down.output_scale
    } else {
        -(pitch_degress
            .abs()
            .min(range_map_vertical_up.input_max_value)
            / range_map_vertical_up.input_max_value
            * range_map_vertical_up.output_scale)
    };
    left_eye.with_rotation(Quat::from_euler(
        EulerRot::YXZ,
        yaw.to_radians(),
        pitch.to_radians(),
        0.0,
    ))
}

fn apply_right_eye_bone(
    right_eye: &Transform,
    properties: &LookAtProperties,
    yaw_degress: f32,
    pitch_degress: f32,
) -> Transform {
    let range_map_horizontal_outer = properties.range_map_horizontal_outer;
    let range_map_horizontal_inner = properties.range_map_horizontal_inner;
    let range_map_vertical_down = properties.range_map_vertical_down;
    let range_map_vertical_up = properties.range_map_vertical_up;

    let yaw = if yaw_degress > 0.0 {
        yaw_degress.min(range_map_horizontal_inner.input_max_value)
            / range_map_horizontal_inner.input_max_value
            * range_map_horizontal_inner.output_scale
    } else {
        -(yaw_degress
            .abs()
            .min(range_map_horizontal_outer.input_max_value)
            / range_map_horizontal_outer.input_max_value
            * range_map_horizontal_outer.output_scale)
    };

    let pitch = if pitch_degress > 0.0 {
        pitch_degress.min(range_map_vertical_down.input_max_value)
            / range_map_vertical_down.input_max_value
            * range_map_vertical_down.output_scale
    } else {
        -(pitch_degress
            .abs()
            .min(range_map_vertical_up.input_max_value)
            / range_map_vertical_up.input_max_value
            * range_map_vertical_up.output_scale)
    };

    right_eye.with_rotation(Quat::from_euler(
        EulerRot::YXZ,
        yaw.to_radians(),
        pitch.to_radians(),
        0.0,
    ))
}
