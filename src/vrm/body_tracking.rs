use crate::prelude::*;
use crate::system_set::VrmSystemSets;
use crate::vrm::look_at::gaze_target::GazeResolveSet;
use crate::vrm::look_at::{calc_yaw_pitch, root_rest_rotation, track_looking_target};
use crate::vrm::{RestGlobalTransform, RestTransform};
use bevy::app::{App, Plugin};
use bevy::prelude::*;
use std::collections::HashMap;

/// Optional body tracking that makes head, neck, chest, and spine bones
/// follow the `LookAt` target. Insert alongside [`LookAt`] to enable.
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
///
/// fn spawn(mut commands: Commands, asset_server: Res<AssetServer>) {
///     commands.spawn((
///         VrmHandle(asset_server.load("model.vrm")),
///         LookAt::Cursor,
///         BodyTracking::default(),
///     ));
/// }
/// ```
#[derive(Component, Debug, Clone, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct BodyTracking {
    /// Fraction of total gaze angle applied to head bone (0.0-1.0).
    pub head_weight: f32,
    /// Fraction of total gaze angle applied to neck bone (0.0-1.0).
    pub neck_weight: f32,
    /// Fraction of total gaze angle applied to chest bone (0.0-1.0).
    pub chest_weight: f32,
    /// Fraction of total gaze angle applied to spine bone (0.0-1.0).
    pub spine_weight: f32,

    /// Maximum head yaw in degrees.
    pub head_yaw_max: f32,
    /// Maximum head pitch in degrees.
    pub head_pitch_max: f32,
    /// Maximum neck yaw in degrees.
    pub neck_yaw_max: f32,
    /// Maximum neck pitch in degrees.
    pub neck_pitch_max: f32,
    /// Maximum chest yaw in degrees.
    pub chest_yaw_max: f32,
    /// Maximum chest pitch in degrees. Set to 0.0 for yaw-only.
    pub chest_pitch_max: f32,
    /// Maximum spine yaw in degrees.
    pub spine_yaw_max: f32,
    /// Maximum spine pitch in degrees. Set to 0.0 for yaw-only.
    pub spine_pitch_max: f32,

    /// Smoothing speed. Higher values = faster response. 0.0 = instant (no smoothing).
    pub smoothing: f32,

    /// Output smoothing speed for suppressing jitter during animation transitions.
    /// Higher values = faster response. 0.0 = instant (no smoothing).
    #[cfg_attr(feature = "serde", serde(default = "default_output_smoothing"))]
    pub output_smoothing: f32,

    /// Minimum forward depth (meters) for yaw/pitch calculation.
    /// Prevents extreme yaw when the camera is directly in front of the model.
    /// Higher values reduce sensitivity. Default: 1.0.
    #[cfg_attr(feature = "serde", serde(default = "default_reference_depth"))]
    pub reference_depth: f32,
}

impl Default for BodyTracking {
    fn default() -> Self {
        Self {
            head_weight: 0.4,
            neck_weight: 0.25,
            chest_weight: 0.2,
            spine_weight: 0.15,
            head_yaw_max: 40.0,
            head_pitch_max: 30.0,
            neck_yaw_max: 25.0,
            neck_pitch_max: 20.0,
            chest_yaw_max: 20.0,
            chest_pitch_max: 0.0,
            spine_yaw_max: 15.0,
            spine_pitch_max: 0.0,
            smoothing: 10.0,
            output_smoothing: 25.0,
            reference_depth: 1.0,
        }
    }
}

fn default_output_smoothing() -> f32 {
    25.0
}

fn default_reference_depth() -> f32 {
    1.0
}

/// Smoothed gaze state stored on the VRM root entity.
/// Inserted automatically when `BodyTracking` is present.
#[derive(Component, Debug, Clone, Reflect, Default)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SmoothedGaze {
    pub yaw: f32,
    pub pitch: f32,
}

pub(super) struct BodyTrackingPlugin;

impl Plugin for BodyTrackingPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<BodyTracking>()
            .register_type::<SmoothedGaze>()
            .add_observer(auto_insert_smoothed_gaze)
            .add_systems(
                PostUpdate,
                track_body_tracking
                    .in_set(VrmSystemSets::GazeControl)
                    .after(VrmSystemSets::PropagateAfterConstraints)
                    .after(GazeResolveSet)
                    .before(track_looking_target)
                    .run_if(any_with_component::<BodyTracking>),
            );
    }
}

/// Automatically insert [`SmoothedGaze`] when [`BodyTracking`] is added.
fn auto_insert_smoothed_gaze(
    trigger: On<Insert<BodyTracking>>,
    mut commands: Commands,
) {
    commands
        .entity(trigger.event_target())
        .insert(SmoothedGaze::default());
}

/// Exponential-decay smoothing with shortest-arc delta for angles.
fn smooth_angle(
    current: f32,
    target: f32,
    speed: f32,
    dt: f32,
) -> f32 {
    if speed <= 0.0 {
        return target;
    }
    let mut delta = target - current;
    while delta > 180.0 {
        delta -= 360.0;
    }
    while delta < -180.0 {
        delta += 360.0;
    }
    current + delta * (1.0 - (-speed * dt).exp())
}

/// Compute bone rotation from yaw/pitch using the same formula as eye rotation.
fn bone_rotation(
    yaw_degrees: f32,
    pitch_degrees: f32,
    rest_tf: &RestTransform,
    rest_gtf: &RestGlobalTransform,
) -> Quat {
    (rest_tf.rotation * rest_gtf.rotation().inverse())
        * Quat::from_euler(
            EulerRot::YXZ,
            yaw_degrees.to_radians(),
            pitch_degrees.to_radians(),
            0.0,
        )
        * rest_gtf.rotation()
}

/// Like [`calc_yaw_pitch`] but clamps the local Z component to `min_depth`
/// before computing angles. This prevents extreme yaw values when the
/// cursor projection places the target at the same depth as the head
/// (e.g., camera directly in front of the model).
fn calc_yaw_pitch_clamped(
    look_at_space: &GlobalTransform,
    target: Vec3,
    min_depth: f32,
) -> (f32, f32) {
    let local_target = look_at_space.to_matrix().inverse().transform_point3(target);

    let x = local_target.dot(Vec3::X);
    let y = local_target.dot(Vec3::Y);
    let z = local_target.dot(Vec3::Z).max(min_depth);

    let yaw = (x.atan2(z)).to_degrees();
    let xz = (x * x + z * z).sqrt();
    let pitch = (-y.atan2(xz)).to_degrees();

    (yaw, pitch)
}

/// Per-bone state for tracking animation changes between frames.
#[derive(Debug, Clone)]
struct BoneState {
    /// The bone rotation before body tracking was applied (from animation or rest).
    base: Quat,
    /// Whether state has been initialized.
    initialized: bool,
    /// The final output rotation from last frame. Used for slerp smoothing and
    /// to detect animation writes: anything else in `Transform.rotation` was
    /// written by someone other than us.
    prev_output: Quat,
}

impl Default for BoneState {
    fn default() -> Self {
        Self {
            base: Quat::IDENTITY,
            initialized: false,
            prev_output: Quat::IDENTITY,
        }
    }
}

/// Compute additive rotation: apply the gaze delta (relative to rest) on top of
/// the base (animated) rotation.
///
/// `base` — current bone rotation from animation (or rest if no animation).
/// `rest` — the bone's rest pose local rotation.
/// `gaze` — the target rotation computed by `bone_rotation()`.
///
/// Returns `base * (rest⁻¹ * gaze)`.
/// When `base == rest`, this simplifies to `gaze` (identical to overwrite mode).
fn compute_additive_rotation(
    base: Quat,
    rest: Quat,
    gaze: Quat,
) -> Quat {
    let delta = rest.inverse() * gaze;
    base * delta
}

/// Bone descriptor used when iterating through the chain.
struct BoneEntry {
    entity: Entity,
    weight: f32,
    yaw_max: f32,
    pitch_max: f32,
}

/// The look-at space in which body tracking measures the gaze.
///
/// Its axes follow the avatar root, so the gaze is measured in the avatar's own
/// frame, and its origin is the rest-pose eye position carried by the head.
/// Unlike the eye look-at space it does not follow the head bone's rest tilt.
pub(crate) fn body_look_at_space(
    head_translation: Vec3,
    root_gtf: &GlobalTransform,
    root_rest_rot: Quat,
    head_rest_rot: Quat,
    offset: Vec3,
) -> GlobalTransform {
    let root_rot = root_gtf.rotation();
    let head_in_root = root_rest_rot.inverse() * head_rest_rot;
    GlobalTransform::from(Transform {
        translation: head_translation + root_rot * head_in_root * (root_gtf.scale() * offset),
        rotation: root_rot,
        scale: Vec3::ONE,
    })
}

/// Returns the world transform of `entity`'s parent as of this frame.
///
/// Walks up the `ChildOf` chain to the first ancestor in `computed` (a bone
/// already updated this frame) and composes the local transforms of the
/// untracked bones between it and `entity` on top of its world transform, so
/// bones such as `upperChest` between tracked ones carry this frame's torso
/// turn. Without such an ancestor, falls back to the stored `GlobalTransform`
/// of the direct parent, read from `root_gtfs` when that parent is the `Vrm`
/// root.
fn parent_world_this_frame(
    entity: Entity,
    computed: &[(Entity, GlobalTransform)],
    child_ofs: &Query<&ChildOf>,
    transforms: &Query<(&mut Transform, &mut GlobalTransform), (Without<Camera>, Without<Vrm>)>,
    root_gtfs: &Query<&GlobalTransform, With<Vrm>>,
) -> Option<GlobalTransform> {
    let direct_parent = child_ofs.get(entity).ok()?.parent();
    // Local transforms of the untracked bones, nearest to `entity` first.
    let mut between: Vec<Transform> = Vec::new();
    let mut current = direct_parent;
    loop {
        if let Some((_, gtf)) = computed.iter().rev().find(|(e, _)| *e == current) {
            return Some(
                between
                    .iter()
                    .rev()
                    .fold(*gtf, |world, local| world.mul_transform(*local)),
            );
        }
        let Ok((tf, _)) = transforms.get(current) else {
            break;
        };
        between.push(*tf);
        let Ok(parent) = child_ofs.get(current) else {
            break;
        };
        current = parent.parent();
    }
    transforms
        .get(direct_parent)
        .map(|(_, gtf)| *gtf)
        .ok()
        .or_else(|| root_gtfs.get(direct_parent).ok().copied())
}

fn track_body_tracking(
    mut vrms: Query<
        (
            Entity,
            &GazeTarget,
            &LookAtProperties,
            &BodyTracking,
            &HeadBoneEntity,
            Option<&NeckBoneEntity>,
            Option<&ChestBoneEntity>,
            Option<&SpineBoneEntity>,
            &mut SmoothedGaze,
        ),
        With<LookAt>,
    >,
    mut transforms: Query<(&mut Transform, &mut GlobalTransform), (Without<Camera>, Without<Vrm>)>,
    root_gtfs: Query<&GlobalTransform, With<Vrm>>,
    child_ofs: Query<&ChildOf>,
    rests: Query<(&RestTransform, &RestGlobalTransform)>,
    time: Res<Time>,
    mut bone_states: Local<HashMap<Entity, BoneState>>,
) {
    let dt = time.delta_secs();

    for (root_entity, gaze, properties, tracking, head, neck, chest, spine, mut smoothed) in
        vrms.iter_mut()
    {
        let Ok((&head_tf, &head_gtf)) = transforms.get(head.0) else {
            continue;
        };
        let Ok((rest_tf, rest_gtf)) = rests.get(head.0) else {
            continue;
        };
        let Ok(root_gtf) = root_gtfs.get(root_entity) else {
            continue;
        };

        let root_rest_rot = root_rest_rotation(&rests, root_entity);
        let look_at_space = body_look_at_space(
            head_gtf.translation(),
            root_gtf,
            root_rest_rot,
            rest_gtf.rotation(),
            Vec3::from(properties.offset_from_head_bone),
        );

        // 2. Calculate raw yaw/pitch with z-depth clamping.
        let (raw_yaw, raw_pitch) =
            calc_yaw_pitch_clamped(&look_at_space, gaze.position(), tracking.reference_depth);

        // 3. Smooth yaw/pitch.
        smoothed.yaw = smooth_angle(smoothed.yaw, raw_yaw, tracking.smoothing, dt);
        smoothed.pitch = smooth_angle(smoothed.pitch, raw_pitch, tracking.smoothing, dt);

        // Normalize yaw to [-180, 180] to prevent unbounded growth
        // during ±180° crossings when cursor passes over the head.
        while smoothed.yaw > 180.0 {
            smoothed.yaw -= 360.0;
        }
        while smoothed.yaw < -180.0 {
            smoothed.yaw += 360.0;
        }

        // 4. Build bone chain bottom-up: spine -> chest -> neck -> head.
        let mut chain: Vec<BoneEntry> = Vec::with_capacity(4);
        if let Some(spine) = spine {
            chain.push(BoneEntry {
                entity: spine.0,
                weight: tracking.spine_weight,
                yaw_max: tracking.spine_yaw_max,
                pitch_max: tracking.spine_pitch_max,
            });
        }
        if let Some(chest) = chest {
            chain.push(BoneEntry {
                entity: chest.0,
                weight: tracking.chest_weight,
                yaw_max: tracking.chest_yaw_max,
                pitch_max: tracking.chest_pitch_max,
            });
        }
        if let Some(neck) = neck {
            chain.push(BoneEntry {
                entity: neck.0,
                weight: tracking.neck_weight,
                yaw_max: tracking.neck_yaw_max,
                pitch_max: tracking.neck_pitch_max,
            });
        }
        chain.push(BoneEntry {
            entity: head.0,
            weight: tracking.head_weight,
            yaw_max: tracking.head_yaw_max,
            pitch_max: tracking.head_pitch_max,
        });

        // 5. Apply rotations bottom-up, propagating GlobalTransform along the chain.
        //    We track which entities we've already computed so we can use local
        //    cached values for parent lookups when the parent is also in the chain.
        let mut computed_gtfs: Vec<(Entity, GlobalTransform)> = Vec::with_capacity(4);

        for bone in &chain {
            let Ok((rest_tf, rest_gtf)) = rests.get(bone.entity) else {
                continue;
            };

            let bone_yaw = (smoothed.yaw * bone.weight).clamp(-bone.yaw_max, bone.yaw_max);
            let bone_pitch = (smoothed.pitch * bone.weight).clamp(-bone.pitch_max, bone.pitch_max);
            let rotation = bone_rotation(bone_yaw, bone_pitch, rest_tf, rest_gtf);

            // The parent's world transform as of this frame, including any
            // untracked bones between this bone and the nearest chain ancestor.
            let Some(parent_gtf) = parent_world_this_frame(
                bone.entity,
                &computed_gtfs,
                &child_ofs,
                &transforms,
                &root_gtfs,
            ) else {
                continue;
            };

            // Write Transform.rotation additively and propagate GlobalTransform.
            let Ok((mut tf, mut gtf)) = transforms.get_mut(bone.entity) else {
                continue;
            };

            let state = bone_states.entry(bone.entity).or_default();

            // Detect whether animation updated the rotation since last frame.
            // We only ever leave `prev_output` in `Transform.rotation`, so any
            // other value was written by the animation system (however small
            // the change) and becomes the new base. Comparing against the
            // unsmoothed `base * delta` would misread our own smoothed output
            // as an animation write and drift.
            let base = if !state.initialized || tf.rotation != state.prev_output {
                tf.rotation
            } else {
                state.base
            };

            let target = compute_additive_rotation(base, rest_tf.rotation, rotation);

            tf.rotation = if state.initialized && tracking.output_smoothing > 0.0 {
                let factor = 1.0 - (-tracking.output_smoothing * dt).exp();
                let prev = if state.prev_output.dot(target) < 0.0 {
                    -state.prev_output
                } else {
                    state.prev_output
                };
                prev.slerp(target, factor)
            } else {
                target
            };

            state.base = base;
            state.prev_output = tf.rotation;
            state.initialized = true;

            *gtf = parent_gtf.mul_transform(*tf);
            computed_gtfs.push((bone.entity, *gtf));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{rotation, rotation_is, run_frames, test_app};
    use crate::vrm::look_at::cursor::CursorRayThisFrame;
    use crate::vrm::look_at::eye_look_at_space;
    use crate::vrm::look_at::gaze_target::resolve_gaze_target;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::time::TimeUpdateStrategy;
    use std::f32::consts::{FRAC_PI_2, PI};
    use std::time::Duration;

    /// In world space; `(-0.5, 0, 2)` from the head in the frame of an avatar
    /// turned by `PI` about `Y`.
    const TARGET: Vec3 = Vec3::new(0.5, 1.5, -2.0);

    /// Runs one tracking pass at [`TARGET`] for an owner turned by `owner`
    /// whose internal scene root carries `scene_root`, and returns the yaw.
    /// `raw` puts `Vrm` on the scene root, as a raw `WorldAssetRoot` spawn
    /// does, instead of on the owner.
    fn tracked_yaw(
        owner: Quat,
        scene_root: Quat,
        raw: bool,
    ) -> f32 {
        let mut app = test_app();
        let owner_tf = Transform::from_rotation(owner);
        let scene_tf = Transform::from_rotation(scene_root);
        let head_tf = Transform::from_xyz(0.0, 1.5, 0.0);
        let owner_gtf = GlobalTransform::from(owner_tf);
        let scene_gtf = owner_gtf.mul_transform(scene_tf);

        let owner_entity = app.world_mut().spawn((owner_tf, owner_gtf)).id();
        let scene = app
            .world_mut()
            .spawn((
                scene_tf,
                scene_gtf,
                ChildOf(owner_entity),
                RestTransform(scene_tf),
                RestGlobalTransform(GlobalTransform::from(scene_tf)),
            ))
            .id();
        let head = app
            .world_mut()
            .spawn((
                head_tf,
                scene_gtf.mul_transform(head_tf),
                ChildOf(scene),
                RestTransform(head_tf),
                RestGlobalTransform(GlobalTransform::from(scene_tf).mul_transform(head_tf)),
            ))
            .id();
        let root = if raw { scene } else { owner_entity };
        app.world_mut().entity_mut(root).insert((
            Vrm,
            LookAt::Cursor,
            GazeTarget::new(TARGET),
            LookAtProperties::default(),
            BodyTracking {
                smoothing: 0.0,
                output_smoothing: 0.0,
                ..default()
            },
            SmoothedGaze::default(),
            HeadBoneEntity(head),
        ));

        app.world_mut()
            .run_system_once(track_body_tracking)
            .expect("Failed to run track_body_tracking");
        app.world().get::<SmoothedGaze>(root).unwrap().yaw
    }

    fn expected_yaw() -> f32 {
        (-0.5_f32).atan2(2.0).to_degrees()
    }

    #[test]
    fn a_turned_avatar_measures_the_gaze_in_its_own_frame() {
        let yaw = tracked_yaw(Quat::from_rotation_y(PI), Quat::IDENTITY, false);
        assert!(
            (yaw - expected_yaw()).abs() < 0.01,
            "yaw {yaw}, expected {}",
            expected_yaw()
        );
    }

    #[test]
    fn a_raw_spawn_measures_the_gaze_in_its_owners_frame() {
        let yaw = tracked_yaw(
            Quat::from_rotation_y(FRAC_PI_2),
            Quat::from_rotation_y(FRAC_PI_2),
            true,
        );
        assert!(
            (yaw - expected_yaw()).abs() < 0.01,
            "yaw {yaw}, expected {}",
            expected_yaw()
        );
    }

    /// A `Vrm` root at the origin with a plain `hips` and a `head` at
    /// `(0, 1.5, 0.3)`, ready for `track_body_tracking`.
    fn spawn_rig(
        app: &mut App,
        extra: impl Bundle,
    ) -> (Entity, Entity) {
        let hips_tf = Transform::from_xyz(0.0, 1.0, 0.3);
        let head_tf = Transform::from_xyz(0.0, 0.5, 0.0);
        let root = app
            .world_mut()
            .spawn((
                Vrm,
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                LookAtProperties {
                    offset_from_head_bone: [0.0; 3],
                    ..default()
                },
                BodyTracking {
                    smoothing: 0.0,
                    output_smoothing: 0.0,
                    ..default()
                },
                extra,
            ))
            .id();
        let hips = app
            .world_mut()
            .spawn((hips_tf, GlobalTransform::from(hips_tf), ChildOf(root)))
            .id();
        let head = app
            .world_mut()
            .spawn((
                head_tf,
                GlobalTransform::from(hips_tf).mul_transform(head_tf),
                ChildOf(hips),
                RestTransform(head_tf),
                RestGlobalTransform(GlobalTransform::from(hips_tf).mul_transform(head_tf)),
            ))
            .id();
        app.world_mut()
            .entity_mut(root)
            .insert(HeadBoneEntity(head));
        (root, head)
    }

    fn track(app: &mut App) {
        app.world_mut()
            .run_system_once(track_body_tracking)
            .expect("Failed to run track_body_tracking");
    }

    #[test]
    fn a_tilted_neck_rest_does_not_bias_body_pitch() {
        let mut app = test_app();
        let hips_tf = Transform::from_xyz(0.0, 1.0, 0.0);
        let neck_tf = Transform::from_xyz(0.0, 0.3, 0.0)
            .with_rotation(Quat::from_rotation_x(6.85_f32.to_radians()));
        let head_tf = Transform::from_xyz(0.0, 0.1, 0.0)
            .with_rotation(Quat::from_rotation_x(-6.23_f32.to_radians()));
        let hips_gtf = GlobalTransform::from(hips_tf);
        let neck_gtf = hips_gtf.mul_transform(neck_tf);
        let head_gtf = neck_gtf.mul_transform(head_tf);
        let root = app
            .world_mut()
            .spawn((
                Vrm,
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                LookAt::Cursor,
                GazeTarget::new(head_gtf.translation() + Vec3::new(0.0, 0.0, 2.0)),
                LookAtProperties {
                    offset_from_head_bone: [0.0; 3],
                    ..default()
                },
                BodyTracking {
                    smoothing: 0.0,
                    output_smoothing: 0.0,
                    ..default()
                },
                SmoothedGaze {
                    yaw: 45.0,
                    pitch: 45.0,
                },
            ))
            .id();
        let hips = app
            .world_mut()
            .spawn((hips_tf, hips_gtf, ChildOf(root)))
            .id();
        let neck = app
            .world_mut()
            .spawn((
                neck_tf,
                neck_gtf,
                ChildOf(hips),
                RestTransform(neck_tf),
                RestGlobalTransform(neck_gtf),
            ))
            .id();
        let head = app
            .world_mut()
            .spawn((
                head_tf,
                head_gtf,
                ChildOf(neck),
                RestTransform(head_tf),
                RestGlobalTransform(head_gtf),
            ))
            .id();
        app.world_mut()
            .entity_mut(root)
            .insert((HeadBoneEntity(head), NeckBoneEntity(neck)));

        track(&mut app);

        let smoothed = app.world().get::<SmoothedGaze>(root).unwrap();
        assert!(smoothed.pitch.abs() < 0.01, "pitch {}", smoothed.pitch);
        assert!(smoothed.yaw.abs() < 0.01, "yaw {}", smoothed.yaw);
    }

    #[test]
    fn the_body_origin_matches_the_rest_eye_origin() {
        let head_rest = RestGlobalTransform(GlobalTransform::from(
            Transform::from_xyz(0.0, 1.4, 0.0)
                .with_rotation(Quat::from_rotation_x(90_f32.to_radians())),
        ));
        let offset = Vec3::new(0.0, 0.06, 0.0);
        let head = Vec3::new(0.0, 1.4, 0.0);
        let body = body_look_at_space(
            head,
            &GlobalTransform::IDENTITY,
            Quat::IDENTITY,
            head_rest.rotation(),
            offset,
        );
        assert!(
            body.translation()
                .abs_diff_eq(Vec3::new(0.0, 1.4, 0.06), 1e-5)
        );
        let eye = eye_look_at_space(&head_rest.0, &head_rest, Quat::IDENTITY, offset);
        assert!(body.translation().abs_diff_eq(eye.translation(), 1e-5));
        let scaled = body_look_at_space(
            head,
            &GlobalTransform::from_scale(Vec3::splat(2.0)),
            Quat::IDENTITY,
            head_rest.rotation(),
            offset,
        );
        assert!(
            scaled
                .translation()
                .abs_diff_eq(Vec3::new(0.0, 1.4, 0.12), 1e-5)
        );
    }

    fn assert_turns_toward_target(
        app: &mut App,
        root: Entity,
    ) {
        app.init_resource::<CursorRayThisFrame>();
        app.world_mut()
            .run_system_once(resolve_gaze_target)
            .expect("Failed to run resolve_gaze_target");
        track(app);
        let expected = 0.5_f32.atan2(2.0 - 0.3).to_degrees();
        let yaw = app.world().get::<SmoothedGaze>(root).unwrap().yaw;
        assert!(
            (yaw - expected).abs() < 0.1,
            "yaw {yaw}, expected {expected}"
        );
    }

    #[test]
    fn a_camera_target_turns_the_body() {
        let mut app = test_app();
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                GlobalTransform::from_xyz(0.5, 1.5, 2.0),
            ))
            .id();
        let (root, _) = spawn_rig(&mut app, (LookAt::Target(camera), SmoothedGaze::default()));
        assert_turns_toward_target(&mut app, root);
    }

    #[test]
    fn another_vrm_target_turns_the_body() {
        let mut app = test_app();
        let other = app
            .world_mut()
            .spawn((Vrm, GlobalTransform::from_xyz(0.5, 1.5, 2.0)))
            .id();
        let (root, _) = spawn_rig(&mut app, (LookAt::Target(other), SmoothedGaze::default()));
        assert_turns_toward_target(&mut app, root);
    }

    #[test]
    fn the_body_does_not_move_without_look_at() {
        let mut app = test_app();
        let (root, head) = spawn_rig(
            &mut app,
            (
                GazeTarget::new(Vec3::new(1.0, 0.0, 1.0)),
                SmoothedGaze {
                    yaw: 45.0,
                    pitch: 45.0,
                },
            ),
        );
        let before = *app.world().get::<Transform>(head).unwrap();

        track(&mut app);

        let smoothed = app.world().get::<SmoothedGaze>(root).unwrap();
        assert_eq!((smoothed.yaw, smoothed.pitch), (45.0, 45.0));
        assert_eq!(*app.world().get::<Transform>(head).unwrap(), before);
    }

    #[test]
    fn test_smooth_angle_converges() {
        let mut current = 0.0;
        for _ in 0..100 {
            current = smooth_angle(current, 45.0, 10.0, 1.0 / 60.0);
        }
        assert!(
            (current - 45.0).abs() < 0.1,
            "Should converge to target: {current}"
        );
    }

    #[test]
    fn test_smooth_angle_instant_when_speed_zero() {
        let result = smooth_angle(0.0, 45.0, 0.0, 1.0 / 60.0);
        assert!((result - 45.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_smooth_angle_shortest_arc() {
        let result = smooth_angle(170.0, -170.0, 100.0, 1.0);
        // Should go through 180 (20 degrees), not through 0 (340 degrees)
        assert!(
            !(-160.0..=170.0).contains(&result),
            "Should take shortest arc: {result}"
        );
    }

    #[test]
    fn test_smooth_angle_no_change_at_target() {
        let result = smooth_angle(45.0, 45.0, 10.0, 1.0 / 60.0);
        assert!((result - 45.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_body_tracking_default_weights_sum_to_one() {
        let bt = BodyTracking::default();
        let total = bt.head_weight + bt.neck_weight + bt.chest_weight + bt.spine_weight;
        assert!(
            (total - 1.0).abs() < f32::EPSILON,
            "Default weights should sum to 1.0: {total}"
        );
    }

    #[test]
    fn test_body_tracking_default_spine_pitch_is_yaw_only() {
        let bt = BodyTracking::default();
        assert_eq!(bt.spine_pitch_max, 0.0);
        assert_eq!(bt.chest_pitch_max, 0.0);
    }

    #[test]
    fn test_bone_rotation_identity_at_zero() {
        let rest_tf = RestTransform(Transform::IDENTITY);
        let rest_gtf = RestGlobalTransform(GlobalTransform::IDENTITY);
        let result = bone_rotation(0.0, 0.0, &rest_tf, &rest_gtf);
        let diff = result.angle_between(Quat::IDENTITY);
        assert!(
            diff < 0.001,
            "Zero yaw/pitch should produce near-identity: {diff}"
        );
    }

    #[test]
    fn test_compute_additive_rotation_no_animation() {
        // When no animation is playing (base == rest), result should equal
        // the full gaze rotation (same as current overwrite behavior).
        let rest = Quat::from_rotation_y(0.3);
        let gaze = bone_rotation(
            15.0,
            10.0,
            &RestTransform(Transform::from_rotation(rest)),
            &RestGlobalTransform(GlobalTransform::from(Transform::from_rotation(rest))),
        );
        let result = compute_additive_rotation(rest, rest, gaze);
        let diff = result.angle_between(gaze);
        assert!(
            diff < 0.001,
            "With no animation (base==rest), should equal gaze rotation: diff={diff}"
        );
    }

    #[test]
    fn test_compute_additive_rotation_with_animation() {
        // When animation sets a different rotation, the gaze delta should be
        // applied on top of the animated rotation, NOT replace it.
        let rest = Quat::IDENTITY;
        let animated = Quat::from_rotation_x(0.2); // Animation tilts spine forward
        let gaze = bone_rotation(
            10.0,
            0.0,
            &RestTransform(Transform::IDENTITY),
            &RestGlobalTransform(GlobalTransform::IDENTITY),
        );
        let result = compute_additive_rotation(animated, rest, gaze);
        // Result should NOT equal gaze (which ignores animation)
        let diff_from_gaze = result.angle_between(gaze);
        assert!(
            diff_from_gaze > 0.01,
            "With animation, result should differ from pure gaze: diff={diff_from_gaze}"
        );
        // Result should incorporate the animation's forward tilt
        let diff_from_animated = result.angle_between(animated);
        assert!(
            diff_from_animated > 0.01,
            "Result should also differ from pure animation: diff={diff_from_animated}"
        );
    }

    #[test]
    fn test_compute_additive_rotation_zero_gaze_preserves_animation() {
        // When gaze is zero (looking straight ahead), the animation rotation
        // should be fully preserved.
        let rest = Quat::from_rotation_y(0.5);
        let animated = Quat::from_rotation_x(0.3) * Quat::from_rotation_y(0.5);
        let gaze_at_zero = bone_rotation(
            0.0,
            0.0,
            &RestTransform(Transform::from_rotation(rest)),
            &RestGlobalTransform(GlobalTransform::from(Transform::from_rotation(rest))),
        );
        // gaze_at_zero == rest, so delta == identity, result == animated
        let result = compute_additive_rotation(animated, rest, gaze_at_zero);
        let diff = result.angle_between(animated);
        assert!(
            diff < 0.001,
            "Zero gaze should preserve animated rotation: diff={diff}"
        );
    }

    /// The head's gaze origin in `spawn_rig`: the head bone, with no offset.
    const RIG_HEAD_WORLD: Vec3 = Vec3::new(0.0, 1.5, 0.3);

    fn head_yaw_degrees(
        app: &App,
        head: Entity,
    ) -> f32 {
        let rotation = app.world().get::<Transform>(head).unwrap().rotation;
        rotation.to_euler(EulerRot::YXZ).0.to_degrees()
    }

    fn set_gaze_target(
        app: &mut App,
        root: Entity,
        target: Vec3,
    ) {
        app.world_mut()
            .entity_mut(root)
            .insert(GazeTarget::new(target));
    }

    #[test]
    fn output_smoothing_settles_on_the_gaze_without_drift() {
        let mut app = test_app();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )));
        app.add_systems(Update, track_body_tracking);
        let (root, head) = spawn_rig(&mut app, (LookAt::Cursor, SmoothedGaze::default()));
        app.world_mut()
            .entity_mut(root)
            .insert(BodyTracking::default());

        let (sin, cos) = 45_f32.to_radians().sin_cos();
        set_gaze_target(
            &mut app,
            root,
            RIG_HEAD_WORLD + Vec3::new(sin, 0.0, cos) * 2.0,
        );
        run_frames(&mut app, 120);
        set_gaze_target(
            &mut app,
            root,
            RIG_HEAD_WORLD + Vec3::new(-sin, 0.0, cos) * 2.0,
        );
        run_frames(&mut app, 600);

        let yaw = head_yaw_degrees(&app, head);
        assert!((yaw + 18.0).abs() < 0.1, "head yaw {yaw}, expected -18.0");
    }

    #[test]
    fn small_animation_changes_become_the_new_base() {
        let mut app = test_app();
        app.add_systems(
            Update,
            (
                |mut frame: Local<u32>, mut tfs: Query<&mut Transform, With<RestTransform>>| {
                    *frame += 1;
                    for mut tf in &mut tfs {
                        tf.rotation = Quat::from_rotation_x((*frame as f32 * 0.5).to_radians());
                    }
                },
                track_body_tracking,
            )
                .chain(),
        );
        let (root, head) = spawn_rig(&mut app, (LookAt::Cursor, SmoothedGaze::default()));
        set_gaze_target(&mut app, root, RIG_HEAD_WORLD + Vec3::new(0.0, 0.0, 2.0));

        for k in 1..=10 {
            app.update();
            let expected = Quat::from_rotation_x((k as f32 * 0.5).to_radians());
            let actual = rotation(&app, head);
            assert!(
                rotation_is(actual, expected),
                "frame {k}: head rotation {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn test_output_smoothing_dampens_base_jump() {
        // When the base (animation) jumps suddenly, the smoothed output should
        // lag behind (i.e. the jump magnitude is reduced in a single frame).
        let rest = Quat::IDENTITY;
        let gaze = bone_rotation(
            10.0,
            0.0,
            &RestTransform(Transform::IDENTITY),
            &RestGlobalTransform(GlobalTransform::IDENTITY),
        );

        let mut state = BoneState::default();
        let dt: f32 = 1.0 / 60.0;
        let output_smoothing: f32 = 25.0;

        // Frame 1: initialize with base == rest
        let target1 = compute_additive_rotation(rest, rest, gaze);
        state.prev_output = target1;
        state.initialized = true;

        // Frame 2: base jumps to a significantly different rotation (simulating VRMA transition)
        let jumped_base = Quat::from_rotation_x(0.5);
        let target2 = compute_additive_rotation(jumped_base, rest, gaze);

        // Apply output smoothing
        let factor = 1.0 - (-output_smoothing * dt).exp();
        let prev = if state.prev_output.dot(target2) < 0.0 {
            -state.prev_output
        } else {
            state.prev_output
        };
        let smoothed = prev.slerp(target2, factor);

        // The smoothed result should be closer to prev_output than the raw target
        let jump_size = state.prev_output.angle_between(target2);
        let smoothed_jump = state.prev_output.angle_between(smoothed);
        assert!(
            smoothed_jump < jump_size,
            "Smoothed jump ({smoothed_jump}) should be less than raw jump ({jump_size})"
        );
    }

    #[test]
    fn test_smoothed_yaw_stays_normalized_during_180_crossing() {
        let mut yaw = 179.0;
        for _ in 0..200 {
            yaw = smooth_angle(yaw, -179.0, 10.0, 1.0 / 60.0);
            while yaw > 180.0 {
                yaw -= 360.0;
            }
            while yaw < -180.0 {
                yaw += 360.0;
            }
        }
        assert!(
            (-180.0..=180.0).contains(&yaw),
            "yaw should stay in [-180, 180]: {yaw}"
        );
        assert!(
            (yaw - (-179.0)).abs() < 0.1,
            "yaw should converge to target: {yaw}"
        );
    }

    #[test]
    fn test_bone_yaw_sign_correct_after_180_crossing() {
        let weight = 0.4;
        let yaw_max = 40.0;
        let mut smoothed_yaw = 179.0;
        for _ in 0..300 {
            smoothed_yaw = smooth_angle(smoothed_yaw, -170.0, 10.0, 1.0 / 60.0);
            while smoothed_yaw > 180.0 {
                smoothed_yaw -= 360.0;
            }
            while smoothed_yaw < -180.0 {
                smoothed_yaw += 360.0;
            }
        }
        let bone_yaw = (smoothed_yaw * weight).clamp(-yaw_max, yaw_max);
        assert!(
            bone_yaw < 0.0,
            "bone_yaw should be negative when target is -170: {bone_yaw}"
        );
    }

    #[test]
    fn test_pitch_always_bounded() {
        for x in [-10.0_f32, -1.0, 0.0, 1.0, 10.0] {
            for z in [-10.0_f32, -0.01, 0.0, 0.01, 10.0] {
                for y in [-10.0_f32, -1.0, 0.0, 1.0, 10.0] {
                    let xz = (x * x + z * z).sqrt();
                    let pitch = (-y.atan2(xz)).to_degrees();
                    assert!(
                        (-90.0..=90.0).contains(&pitch),
                        "pitch out of range: {pitch} for x={x}, y={y}, z={z}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_output_smoothing_disabled_at_zero_speed() {
        // When output_smoothing is 0, the output should snap to target immediately.
        let rest = Quat::IDENTITY;
        let gaze = bone_rotation(
            15.0,
            5.0,
            &RestTransform(Transform::IDENTITY),
            &RestGlobalTransform(GlobalTransform::IDENTITY),
        );

        let state = BoneState {
            prev_output: Quat::from_rotation_y(0.3), // some previous output
            initialized: true,
            ..Default::default()
        };

        let target = compute_additive_rotation(rest, rest, gaze);
        let output_smoothing = 0.0;

        // With speed=0, the branch should take the `else` path (return target directly)
        let result = if state.initialized && output_smoothing > 0.0 {
            unreachable!("Should not enter smoothing branch with speed=0");
        } else {
            target
        };

        let diff = result.angle_between(target);
        assert!(
            diff < 0.001,
            "With speed=0, output should equal target: diff={diff}"
        );
    }

    #[test]
    fn test_calc_yaw_pitch_clamped_zero_depth_target() {
        // When target is at the same Z-depth as the look_at_space origin,
        // the clamped version should produce a small, proportional yaw
        // instead of ~90 degrees.
        let look_at_space = GlobalTransform::from(Transform {
            translation: Vec3::new(0.0, 1.36, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        });
        // Target slightly to the right, at the same Z as origin
        let target = Vec3::new(0.1, 1.3, 0.0);
        let min_depth = 1.0;

        let (yaw, _pitch) = calc_yaw_pitch_clamped(&look_at_space, target, min_depth);
        // With min_depth=1.0, yaw should be atan2(0.1, 1.0) ≈ 5.7 degrees
        assert!(
            yaw.abs() < 10.0,
            "Yaw should be small for slight offset: {yaw}"
        );
        assert!(yaw > 0.0, "Yaw should be positive for right offset: {yaw}");
    }

    #[test]
    fn test_calc_yaw_pitch_clamped_preserves_valid_depth() {
        // When target has meaningful Z depth (> min_depth), the clamp
        // should not activate and the result should match calc_yaw_pitch.
        let look_at_space = GlobalTransform::from(Transform {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        });
        // Target at (1, 0, 3) — well in front, Z=3 > min_depth=1
        let target = Vec3::new(1.0, 0.0, 3.0);
        let min_depth = 1.0;

        let (clamped_yaw, clamped_pitch) =
            calc_yaw_pitch_clamped(&look_at_space, target, min_depth);
        let (original_yaw, original_pitch) = calc_yaw_pitch(&look_at_space, target);

        assert!(
            (clamped_yaw - original_yaw).abs() < 0.01,
            "Should match original when Z > min_depth: clamped={clamped_yaw}, original={original_yaw}"
        );
        assert!(
            (clamped_pitch - original_pitch).abs() < 0.01,
            "Pitch should match: clamped={clamped_pitch}, original={original_pitch}"
        );
    }

    #[test]
    fn test_calc_yaw_pitch_clamped_proportional() {
        // Yaw should be proportional to horizontal offset for small values.
        let look_at_space = GlobalTransform::from(Transform {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        });
        let min_depth = 1.0;

        let (yaw1, _) = calc_yaw_pitch_clamped(&look_at_space, Vec3::new(0.1, 0.0, 0.0), min_depth);
        let (yaw2, _) = calc_yaw_pitch_clamped(&look_at_space, Vec3::new(0.2, 0.0, 0.0), min_depth);

        // yaw2 should be roughly 2x yaw1 for small angles
        let ratio = yaw2 / yaw1;
        assert!(
            (ratio - 2.0).abs() < 0.2,
            "Yaw should be roughly proportional: ratio={ratio}"
        );
    }

    #[test]
    fn test_calc_yaw_pitch_clamped_center_is_zero() {
        // When target is directly in front (along +Z or at origin), yaw should be ~0.
        let look_at_space = GlobalTransform::from(Transform {
            translation: Vec3::new(0.0, 1.3, 0.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        });
        // Target at same position as origin (cursor at face center)
        let target = Vec3::new(0.0, 1.3, 0.0);
        let min_depth = 1.0;

        let (yaw, _pitch) = calc_yaw_pitch_clamped(&look_at_space, target, min_depth);
        assert!(
            yaw.abs() < 0.01,
            "Yaw should be ~0 when target is at origin: {yaw}"
        );
    }

    #[test]
    fn test_calc_yaw_pitch_clamped_behind_model() {
        // When target is behind the model (negative Z), clamp should activate
        // and produce a small, controlled yaw instead of a wild angle.
        let look_at_space = GlobalTransform::from(Transform {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        });
        // Target behind and slightly right
        let target = Vec3::new(0.5, 0.0, -2.0);
        let min_depth = 1.0;

        let (yaw, _pitch) = calc_yaw_pitch_clamped(&look_at_space, target, min_depth);
        // Z clamped from -2.0 to 1.0, so yaw = atan2(0.5, 1.0) ≈ 26.6 degrees
        assert!(
            yaw.abs() < 30.0,
            "Yaw should be moderate for behind-model target: {yaw}"
        );
    }

    struct TorsoRig {
        spine: Entity,
        chest: Entity,
        head: Entity,
        left_eye: Entity,
    }

    const EYE_REST_DEGREES: f32 = 21.8;

    /// `hips -> spine -> chest -> upper_chest -> neck -> head -> eyes`, where
    /// `upper_chest` is not part of the tracked chain. Rests are pure
    /// translations and every `GlobalTransform` is composed by hand because
    /// the test app has no transform propagation.
    fn spawn_torso_rig(app: &mut App) -> TorsoRig {
        let step = Transform::from_xyz(0.0, 0.1, 0.0);
        let hips_tf = Transform::from_xyz(0.0, 0.9, 0.0);
        let head_tf = Transform::from_xyz(0.0, 0.07, 0.0);
        let left_tf = Transform::from_xyz(0.016, 0.056, 0.019)
            .with_rotation(Quat::from_rotation_y(EYE_REST_DEGREES.to_radians()));
        let right_tf = Transform::from_xyz(-0.016, 0.056, 0.019)
            .with_rotation(Quat::from_rotation_y(-EYE_REST_DEGREES.to_radians()));

        let hips_gtf = GlobalTransform::from(hips_tf);
        let spine_gtf = hips_gtf.mul_transform(step);
        let chest_gtf = spine_gtf.mul_transform(step);
        let upper_chest_gtf = chest_gtf.mul_transform(step);
        let neck_gtf = upper_chest_gtf.mul_transform(step);
        let head_gtf = neck_gtf.mul_transform(head_tf);
        let gaze_origin = head_gtf.translation() + Vec3::new(0.0, 0.06, 0.0);

        let range = |max, scale| RangeMap {
            input_max_value: max,
            output_scale: scale,
        };
        let root = app
            .world_mut()
            .spawn((
                Vrm,
                LookAt::Cursor,
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                GazeTarget::new(gaze_origin + Vec3::new(0.3, 0.0, 1.0)),
                LookAtProperties {
                    offset_from_head_bone: [0.0, 0.06, 0.0],
                    range_map_horizontal_inner: range(90.0, 8.0),
                    range_map_horizontal_outer: range(90.0, 12.0),
                    range_map_vertical_down: range(90.0, 10.0),
                    range_map_vertical_up: range(90.0, 10.0),
                    r#type: LookAtType::Bone,
                },
                BodyTracking {
                    smoothing: 0.0,
                    output_smoothing: 0.0,
                    ..default()
                },
                SmoothedGaze::default(),
            ))
            .id();
        let hips = app
            .world_mut()
            .spawn((hips_tf, hips_gtf, ChildOf(root)))
            .id();
        let mut bone = |tf: Transform, gtf: GlobalTransform, parent: Entity| {
            app.world_mut()
                .spawn((
                    tf,
                    gtf,
                    ChildOf(parent),
                    RestTransform(tf),
                    RestGlobalTransform(gtf),
                ))
                .id()
        };
        let spine = bone(step, spine_gtf, hips);
        let chest = bone(step, chest_gtf, spine);
        let upper_chest = bone(step, upper_chest_gtf, chest);
        let neck = bone(step, neck_gtf, upper_chest);
        let head = bone(head_tf, head_gtf, neck);
        let mut eye = |local: Transform| {
            app.world_mut()
                .spawn((
                    Transform::IDENTITY,
                    GlobalTransform::IDENTITY,
                    ChildOf(head),
                    RestTransform(local),
                    RestGlobalTransform(head_gtf.mul_transform(local)),
                ))
                .id()
        };
        let left_eye = eye(left_tf);
        let right_eye = eye(right_tf);
        app.world_mut().entity_mut(root).insert((
            SpineBoneEntity(spine),
            ChestBoneEntity(chest),
            NeckBoneEntity(neck),
            HeadBoneEntity(head),
            LeftEyeBoneEntity(left_eye),
            RightEyeBoneEntity(right_eye),
        ));
        TorsoRig {
            spine,
            chest,
            head,
            left_eye,
        }
    }

    fn assert_head_yaw_and_centred_eyes(
        app: &App,
        rig: &TorsoRig,
    ) {
        let head = app.world().get::<GlobalTransform>(rig.head).unwrap();
        let (yaw, _, _) = head.rotation().to_euler(EulerRot::YXZ);
        let expected = 0.3_f32.atan2(1.0).to_degrees();
        assert!(
            (yaw.to_degrees() - expected).abs() < 0.3,
            "head yaw {} expected {expected}",
            yaw.to_degrees()
        );
        let left = app.world().get::<Transform>(rig.left_eye).unwrap().rotation;
        let rest = Quat::from_rotation_y(EYE_REST_DEGREES.to_radians());
        let deviation = left.angle_between(rest).to_degrees();
        assert!(deviation < 0.2, "left eye deviates {deviation} degrees");
    }

    #[test]
    fn an_upper_chest_between_chest_and_neck_carries_this_frames_torso_turn() {
        let mut app = test_app();
        let rig = spawn_torso_rig(&mut app);
        app.add_systems(Update, (track_body_tracking, track_looking_target).chain());
        app.update();
        assert_head_yaw_and_centred_eyes(&app, &rig);
    }

    #[test]
    fn an_animated_torso_still_leaves_the_eyes_centred() {
        let mut app = test_app();
        let rig = spawn_torso_rig(&mut app);
        let (spine, chest) = (rig.spine, rig.chest);
        let animate = move |mut transforms: Query<&mut Transform>| {
            for bone in [spine, chest] {
                transforms.get_mut(bone).unwrap().rotation = Quat::IDENTITY;
            }
        };
        app.add_systems(
            Update,
            (animate, track_body_tracking, track_looking_target).chain(),
        );
        for _ in 0..3 {
            app.update();
            assert_head_yaw_and_centred_eyes(&app, &rig);
        }
    }
}
