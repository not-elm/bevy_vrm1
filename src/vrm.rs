pub mod body_tracking;
pub mod components;
pub mod detach;
pub(crate) mod expressions;
pub(crate) mod first_person;
pub(crate) mod gltf;
pub(crate) mod humanoid_bone;
pub(crate) mod instance;
mod look_at;
mod mtoon;
pub(crate) mod runtime;
pub mod spring_bone;

use crate::macros::marker_component;
use crate::new_type;
use crate::system_set::VrmSystemSets;
use crate::vrm::body_tracking::BodyTrackingPlugin;
use crate::vrm::detach::VrmDetachPlugin;
use crate::vrm::humanoid_bone::VrmHumanoidBonePlugin;
use crate::vrm::instance::VrmInstancePlugin;
use crate::vrm::look_at::LookAtPlugin;
use crate::vrm::spring_bone::VrmSpringBonePlugin;
use bevy::app::{AnimationSystems, App, Plugin};
use bevy::prelude::*;
use bevy::transform::systems::{
    mark_dirty_trees, propagate_parent_transforms, sync_simple_transforms,
};
use expressions::VrmExpressionPlugin;
use first_person::VrmFirstPersonPlugin;
use gltf::VrmGltfPlugin;
use mtoon::MtoonMaterialPlugin;
use std::path::PathBuf;

pub mod prelude {
    pub use crate::vrm::{
        Initialized, RestGlobalTransform, RestTransform, RestWorldTransform,
        SkipPostSpringPropagation, Vrm, VrmBone, VrmExpression, VrmPath, VrmPlugin,
        body_tracking::{BodyTracking, SmoothedGaze},
        components::{
            ConstraintExecutionOrder, PendingNodeConstraint, VrmConstraintKind, VrmHeadOnly,
            VrmNodeConstraint, VrmNodeIndex,
        },
        detach::RequestDetachVrm,
        expressions::{
            ClearExpressions, ExpressionOverrideType, ModifyExpressions, SetExpressions,
        },
        first_person::{
            FirstPersonCamera, FirstPersonLayers, RequestDisableFirstPerson,
            RequestEnableFirstPerson, ThirdPersonCamera,
        },
        gltf::prelude::*,
        humanoid_bone::prelude::*,
        instance::VrmHandle,
        look_at::{CursorGazeDepth, GazeTarget, LookAt},
        mtoon::prelude::*,
        spring_bone::{SpringJointProps, SpringJoints, SpringRoot},
    };
}

new_type!(
    /// The bone name obtained from `VRMC_vrm::humanoid`.
    name: VrmBone,
    ty: String,
);

new_type!(
    /// The key name of `VRMC_vrm::expressions::preset`.
    name: VrmExpression,
    ty: String,
);

/// A marker component attached to the entity of VRM.
/// This component is automatically inserted after the [`VrmHandle`](crate::prelude::VrmHandle) is loaded.
#[derive(Debug, Component, Reflect, Copy, Clone)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct Vrm;

impl Vrm {
    pub const ROOT_BONE: &'static str = "VRMC_vrm.root_bone";
}

/// The path to the VRM file.
/// This component is automatically inserted after the [`VrmHandle`](crate::prelude::VrmHandle) is loaded.
#[derive(Debug, Reflect, Clone, Component)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct VrmPath(pub PathBuf);

impl VrmPath {
    /// Creates a new [`VrmPath`] from the path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }
}

/// The bone's initial transform.
#[derive(Debug, Copy, Clone, Component, Deref, Reflect, Default)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct RestTransform(pub Transform);

/// The bone's initial global transform.
#[derive(Debug, Copy, Clone, Component, Deref, Reflect, Default)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct RestGlobalTransform(pub GlobalTransform);

/// The world transform of the model entity (VRM or VRMA root) at the moment
/// the rest transforms of its subtree were snapshotted.
///
/// The VRMA retarget composes source and destination rest-global
/// rotations. Those two snapshots happen at different times — the VRM
/// initializes before its VRMA children — and any world rotation the body
/// gains in between would conjugate the baked pose (a constant per-instance
/// limb offset). Stripping this prefix at retarget-table time puts both
/// rigs into the model entity's own frame and removes that dependency.
/// [`RestGlobalTransform`] is not world-space either: it is relative to the
/// entity that owns the scene, and gaze and body tracking re-apply that
/// entity's current world rotation.
#[derive(Debug, Copy, Clone, Component, Deref, Reflect, Default)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct RestWorldTransform(pub GlobalTransform);

marker_component!(
    /// A marker component attached to the entity of VRM.
    /// This component is automatically inserted after the [`VrmHandle`](crate::prelude::VrmHandle) is loaded.
    Initialized
);

/// Skips the extra transform propagation chain that `VrmPlugin` runs after
/// [`VrmSystemSets::SpringBone`].
///
/// Insert this resource to avoid paying for that chain when the host already
/// has a suitable propagation pass of its own, or when nodes below a spring
/// joint that are not simulated do not need same-frame `GlobalTransform`.
/// Spring joints publish their own world transforms at the spring step, so
/// they stay in sync; only unsimulated descendants lag by one frame and catch
/// up during the next frame's propagation. Without this resource the plugin's
/// update order is unchanged.
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy_vrm1::prelude::*;
/// let mut app = App::new();
/// app.insert_resource(SkipPostSpringPropagation);
/// app.add_plugins((DefaultPlugins, VrmPlugin));
/// ```
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SkipPostSpringPropagation;

fn post_spring_propagation_enabled(skip: Option<Res<SkipPostSpringPropagation>>) -> bool {
    skip.is_none()
}

/// The main plugin for VRM support in Bevy.
///
/// Please refer to [`VrmHandle`](crate::prelude::VrmHandle) for more details.
/// Must be added after `DefaultPlugins`, because it registers the `.vrm` loader with `GltfPlugin`.
pub struct VrmPlugin;

impl Plugin for VrmPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_plugins((
            VrmDetachPlugin,
            VrmSpringBonePlugin,
            VrmHumanoidBonePlugin,
            VrmExpressionPlugin,
            VrmInstancePlugin,
            MtoonMaterialPlugin,
            LookAtPlugin,
            BodyTrackingPlugin,
            VrmFirstPersonPlugin,
            VrmGltfPlugin,
        ));

        // Add manual transform propagation systems to follow VRM spec update order
        // See: https://vrm.dev/api/api_update/
        app.add_systems(
            PostUpdate,
            (sync_simple_transforms, propagate_parent_transforms)
                .chain()
                .in_set(VrmSystemSets::PropagateAfterConstraints)
                .after(VrmSystemSets::Constraints)
                .before(VrmSystemSets::GazeControl),
        );
        app.add_systems(
            PostUpdate,
            (sync_simple_transforms, propagate_parent_transforms)
                .chain()
                .in_set(VrmSystemSets::PropagateAfterExpressions)
                .after(VrmSystemSets::Expressions)
                .before(VrmSystemSets::SpringBone),
        );
        app.add_systems(
            PostUpdate,
            (
                mark_dirty_trees,
                propagate_parent_transforms,
                sync_simple_transforms,
            )
                .chain()
                .after(VrmSystemSets::SpringBone)
                .run_if(post_spring_propagation_enabled),
        );

        app.register_type::<Vrm>()
            .register_type::<VrmPath>()
            .register_type::<RestTransform>()
            .register_type::<RestGlobalTransform>()
            .register_type::<RestWorldTransform>()
            .register_type::<VrmBone>()
            .register_type::<VrmExpression>()
            .register_type::<Initialized>();
    }
}

/// The rotation of the model root's rest pose, or identity when it has none.
///
/// [`RestGlobalTransform`] is relative to the entity that owns the scene, so
/// the rest rotation of the root (for example the rotation of a raw-spawned
/// scene root) has to be re-applied when measuring in world space.
///
/// Private to this module so that only its child modules (`look_at` and
/// `body_tracking`) can use it.
fn root_rest_rotation(
    rests: &Query<(&RestTransform, &RestGlobalTransform)>,
    root: Entity,
) -> Quat {
    rests
        .get(root)
        .map_or(Quat::IDENTITY, |(_, rest_gtf)| rest_gtf.rotation())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::SkipPostSpringPropagation;
    use bevy::app::PluginGroupBuilder;
    use bevy::render::RenderPlugin;
    use bevy::render::settings::WgpuSettings;
    use std::f32::consts::FRAC_PI_2;

    fn headless_default_plugins() -> PluginGroupBuilder {
        DefaultPlugins
            .build()
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::log::LogPlugin>()
            .set(RenderPlugin {
                render_creation: WgpuSettings {
                    backends: None,
                    ..default()
                }
                .into(),
                ..default()
            })
    }

    /// The spring step moves a chain's joints after bevy has propagated, and
    /// publishes only the joints it simulates. The chain's last joint has to be
    /// published from its parent's *new* rotation in the same frame, or the tip
    /// of every strand renders one frame behind.
    #[test]
    fn a_spring_chain_tail_follows_its_joint_in_the_same_frame() {
        use crate::vrm::spring_bone::{
            SpringCenterNode, SpringColliders, SpringJointProps, SpringJointState, SpringJoints,
            SpringRoot,
        };
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;

        let mut app = App::new();
        app.add_plugins((headless_default_plugins(), VrmPlugin));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        app.finish();
        app.cleanup();
        let props = SpringJointProps {
            drag_force: 0.0,
            gravity_dir: Vec3::NEG_X,
            gravity_power: 50.0,
            hit_radius: 0.0,
            stiffness: 0.0,
        };
        let rest = Transform::from_xyz(0.0, 1.0, 0.0);
        let root = app.world_mut().spawn(Transform::default()).id();
        let joint = app.world_mut().spawn((rest, ChildOf(root))).id();
        let tail = app
            .world_mut()
            .spawn((Transform::from_xyz(0.0, 1.0, 0.0), ChildOf(joint), props))
            .id();
        app.world_mut()
            .entity_mut(joint)
            .insert((props, SpringJointState::from_rest(Vec3::Y, 1.0, rest)));
        app.world_mut().entity_mut(root).insert(SpringRoot {
            joints: SpringJoints(vec![joint, tail]),
            colliders: SpringColliders::default(),
            center_node: SpringCenterNode(None),
        });

        let mut moved = false;
        for _ in 0..4 {
            app.update();
            let world = app.world();
            let joint_gtf = *world.get::<GlobalTransform>(joint).unwrap();
            let tail_tf = *world.get::<Transform>(tail).unwrap();
            let published = world.get::<GlobalTransform>(tail).unwrap().translation();
            let expected = joint_gtf.mul_transform(tail_tf).translation();
            moved |= joint_gtf.rotation().angle_between(Quat::IDENTITY) > 0.1;
            assert!(
                published.abs_diff_eq(expected, 1e-4),
                "the tail is published at {published:?}, its joint puts it at {expected:?}"
            );
        }
        assert!(
            moved,
            "gravity must actually swing the joint for the test to mean anything"
        );
    }

    struct SpringChainScene {
        app: App,
        root: Entity,
        head: Entity,
        eye: Entity,
        joint: Entity,
        tail: Entity,
        leaf: Entity,
    }

    fn spring_chain_scene(skip_post_spring: bool) -> SpringChainScene {
        use crate::vrm::spring_bone::{
            SpringCenterNode, SpringColliders, SpringJointProps, SpringJointState, SpringJoints,
            SpringRoot,
        };
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;

        let mut app = App::new();
        app.add_plugins((headless_default_plugins(), VrmPlugin));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        if skip_post_spring {
            app.insert_resource(SkipPostSpringPropagation);
        }
        app.finish();
        app.cleanup();

        let props = SpringJointProps {
            drag_force: 0.0,
            gravity_dir: Vec3::NEG_X,
            gravity_power: 50.0,
            hit_radius: 0.0,
            stiffness: 0.0,
        };
        let rest = Transform::from_xyz(0.0, 1.0, 0.0);
        let root = app.world_mut().spawn(Transform::default()).id();
        let head = app.world_mut().spawn((rest, ChildOf(root))).id();
        let eye = app
            .world_mut()
            .spawn((Transform::from_xyz(0.0, 0.1, 0.0), ChildOf(head)))
            .id();
        let joint = app.world_mut().spawn((rest, ChildOf(head))).id();
        let tail = app.world_mut().spawn((rest, ChildOf(joint))).id();
        let leaf = app.world_mut().spawn((rest, ChildOf(tail))).id();
        app.world_mut()
            .entity_mut(joint)
            .insert((props, SpringJointState::from_rest(Vec3::Y, 1.0, rest)));
        app.world_mut()
            .entity_mut(tail)
            .insert((props, SpringJointState::from_rest(Vec3::Y, 1.0, rest)));
        app.world_mut().entity_mut(root).insert(SpringRoot {
            joints: SpringJoints(vec![joint, tail]),
            colliders: SpringColliders::default(),
            center_node: SpringCenterNode(None),
        });
        SpringChainScene {
            app,
            root,
            head,
            eye,
            joint,
            tail,
            leaf,
        }
    }

    fn center_space_spring_scene(skip_post_spring: bool) -> SpringChainScene {
        use crate::vrm::spring_bone::{
            SpringCenterNode, SpringColliders, SpringJointProps, SpringJoints, SpringRoot,
        };

        let mut scene = spring_chain_scene(skip_post_spring);
        let joint = scene.joint;
        let tail = scene.tail;
        let props = SpringJointProps {
            drag_force: 0.0,
            gravity_dir: Vec3::NEG_X,
            gravity_power: 0.0,
            hit_radius: 0.0,
            stiffness: 0.0,
        };
        scene.app.world_mut().entity_mut(joint).insert(props);
        scene.app.world_mut().entity_mut(tail).insert(props);
        scene
            .app
            .world_mut()
            .entity_mut(scene.root)
            .insert(SpringRoot {
                joints: SpringJoints(vec![joint, tail]),
                colliders: SpringColliders::default(),
                center_node: SpringCenterNode(Some(scene.root)),
            });
        scene
    }

    fn center_relative(
        app: &App,
        center: Entity,
        entity: Entity,
    ) -> Vec3 {
        let relative = global_transform(app, center).to_matrix().inverse()
            * global_transform(app, entity).to_matrix();
        relative.w_axis.truncate()
    }

    fn install_collider(
        scene: &mut SpringChainScene,
        position: Vec3,
    ) -> Entity {
        use crate::prelude::{ColliderShape, Sphere};
        use crate::vrm::spring_bone::{SpringColliders, SpringRoot};

        let collider = scene
            .app
            .world_mut()
            .spawn((
                Transform::from_translation(position),
                GlobalTransform::from_translation(position),
            ))
            .id();
        let mut spring_root = scene
            .app
            .world()
            .get::<SpringRoot>(scene.root)
            .unwrap()
            .clone();
        spring_root.colliders = SpringColliders(vec![(
            collider,
            ColliderShape::Sphere(Sphere {
                offset: [0.0; 3],
                radius: 1.5,
            }),
        )]);
        scene
            .app
            .world_mut()
            .entity_mut(scene.root)
            .insert(spring_root);
        collider
    }

    fn transform(
        app: &App,
        entity: Entity,
    ) -> Transform {
        *app.world().get::<Transform>(entity).unwrap()
    }

    fn global_transform(
        app: &App,
        entity: Entity,
    ) -> GlobalTransform {
        *app.world().get::<GlobalTransform>(entity).unwrap()
    }

    fn quat_angle(
        a: Quat,
        b: Quat,
    ) -> f32 {
        2.0 * a.dot(b).abs().clamp(-1.0, 1.0).acos()
    }

    fn quat_eq(
        a: Quat,
        b: Quat,
    ) -> bool {
        a.to_array()
            .iter()
            .zip(b.to_array())
            .all(|(x, y)| (x - y).abs() < 1e-4)
    }

    fn assert_gtf_eq(
        actual: GlobalTransform,
        expected: GlobalTransform,
        label: &str,
    ) {
        assert!(
            actual
                .translation()
                .abs_diff_eq(expected.translation(), 1e-4)
                && quat_eq(actual.rotation(), expected.rotation()),
            "{label}: actual {actual:?}, expected {expected:?}"
        );
    }

    fn assert_child_is_published(
        app: &App,
        parent: Entity,
        child: Entity,
        label: &str,
    ) {
        let expected = global_transform(app, parent).mul_transform(transform(app, child));
        assert_gtf_eq(global_transform(app, child), expected, label);
    }

    fn assert_spring_reads_fresh_parent_rotation(skip_post_spring: bool) {
        let mut moved = spring_chain_scene(skip_post_spring);
        let mut still = spring_chain_scene(skip_post_spring);
        moved.app.update();
        still.app.update();
        let before = global_transform(&moved.app, moved.head);
        let rotation = Quat::from_rotation_z(FRAC_PI_2);
        moved
            .app
            .world_mut()
            .entity_mut(moved.head)
            .get_mut::<Transform>()
            .unwrap()
            .rotation = rotation;
        moved.app.update();
        still.app.update();

        let head = global_transform(&moved.app, moved.head);
        assert!(
            quat_eq(head.rotation(), rotation),
            "the parent's GlobalTransform must follow its Transform: {head:?}"
        );
        assert!(
            head.translation().abs_diff_eq(before.translation(), 1e-4),
            "a pure rotation must not move the parent: {head:?}"
        );
        assert_child_is_published(
            &moved.app,
            moved.head,
            moved.joint,
            "joint after parent rotation",
        );
        assert_child_is_published(
            &moved.app,
            moved.joint,
            moved.tail,
            "tail after parent rotation",
        );
        assert!(
            quat_angle(
                transform(&moved.app, moved.joint).rotation,
                transform(&still.app, still.joint).rotation
            ) > 1e-2,
            "the parent's new world rotation must reach the joint's spring evaluation"
        );
        assert!(
            quat_angle(
                transform(&moved.app, moved.tail).rotation,
                transform(&still.app, still.tail).rotation
            ) > 1e-2,
            "the parent's new world rotation must reach the tail's spring evaluation"
        );
    }

    fn assert_non_spring_bones_are_fresh(skip_post_spring: bool) {
        let mut scene = spring_chain_scene(skip_post_spring);
        scene.app.update();
        let before_head = global_transform(&scene.app, scene.head);
        let before_eye = global_transform(&scene.app, scene.eye);
        scene
            .app
            .world_mut()
            .entity_mut(scene.head)
            .get_mut::<Transform>()
            .unwrap()
            .rotation = Quat::from_rotation_z(FRAC_PI_2);
        scene.app.update();

        assert_gtf_eq(
            global_transform(&scene.app, scene.head),
            GlobalTransform::from(transform(&scene.app, scene.head)),
            "non-spring head",
        );
        assert!(
            quat_angle(
                global_transform(&scene.app, scene.head).rotation(),
                before_head.rotation()
            ) > 1.0,
            "the head must publish its new rotation in the same frame"
        );
        assert_child_is_published(&scene.app, scene.head, scene.eye, "non-spring eye");
        assert!(
            !global_transform(&scene.app, scene.eye)
                .translation()
                .abs_diff_eq(before_eye.translation(), 1e-4),
            "the eye's world position must follow the head in the same frame"
        );
    }

    #[test]
    fn without_post_spring_chain_spring_uses_fresh_parent_rotation_on_next_frame() {
        assert_spring_reads_fresh_parent_rotation(true);
    }

    #[test]
    fn with_post_spring_chain_spring_uses_fresh_parent_rotation_on_next_frame() {
        assert_spring_reads_fresh_parent_rotation(false);
    }

    #[test]
    fn without_post_spring_chain_spring_reads_fresh_collider_world_matrix_on_next_frame() {
        let mut moved = spring_chain_scene(true);
        let mut still = spring_chain_scene(true);
        let moved_collider = install_collider(&mut moved, Vec3::new(-0.5, 3.0, 0.0));
        install_collider(&mut still, Vec3::new(-0.5, 3.0, 0.0));
        moved.app.update();
        still.app.update();

        moved
            .app
            .world_mut()
            .entity_mut(moved_collider)
            .get_mut::<Transform>()
            .unwrap()
            .translation = Vec3::new(10.0, 10.0, 10.0);
        moved.app.update();
        still.app.update();

        assert_gtf_eq(
            global_transform(&moved.app, moved_collider),
            GlobalTransform::from(transform(&moved.app, moved_collider)),
            "moved collider",
        );
        assert!(
            quat_angle(
                transform(&moved.app, moved.joint).rotation,
                transform(&still.app, still.joint).rotation
            ) > 1e-2,
            "the spring must react to the collider's new world matrix"
        );
    }

    #[test]
    fn without_post_spring_chain_non_spring_head_and_eye_global_transforms_are_fresh() {
        assert_non_spring_bones_are_fresh(true);
    }

    #[test]
    fn with_post_spring_chain_non_spring_head_and_eye_global_transforms_are_fresh() {
        assert_non_spring_bones_are_fresh(false);
    }

    /// The leaf is not a spring joint, so only propagation publishes it; the
    /// spring step moves its parent after Bevy's propagation has already run.
    #[test]
    fn without_post_spring_chain_unsimulated_leaf_global_transform_is_stale_for_the_frame() {
        let mut scene = spring_chain_scene(true);
        scene.app.update();

        let actual = global_transform(&scene.app, scene.leaf);
        let expected = global_transform(&scene.app, scene.tail)
            .mul_transform(transform(&scene.app, scene.leaf));
        assert!(
            !actual
                .translation()
                .abs_diff_eq(expected.translation(), 1e-4),
            "the leaf must lag the spring step: actual {actual:?}, current parent {expected:?}"
        );
        assert_child_is_published(&scene.app, scene.joint, scene.tail, "simulated tail");

        let tail_after_first_frame = global_transform(&scene.app, scene.tail);
        scene.app.update();
        let actual = global_transform(&scene.app, scene.leaf);
        let expected = tail_after_first_frame.mul_transform(transform(&scene.app, scene.leaf));
        assert_gtf_eq(actual, expected, "leaf after the next frame's propagation");
    }

    #[test]
    fn with_post_spring_chain_unsimulated_leaf_global_transform_is_fresh() {
        let mut scene = spring_chain_scene(false);
        scene.app.update();
        assert_child_is_published(&scene.app, scene.tail, scene.leaf, "leaf");
    }

    #[test]
    fn post_spring_chain_enabled_and_disabled_agree_on_spring_joints_and_non_spring_bones() {
        let mut enabled = spring_chain_scene(false);
        let mut disabled = spring_chain_scene(true);
        for frame in 0..3 {
            enabled.app.update();
            disabled.app.update();
            for ((on, off), label) in [
                ((enabled.head, disabled.head), "head"),
                ((enabled.eye, disabled.eye), "eye"),
                ((enabled.joint, disabled.joint), "joint"),
                ((enabled.tail, disabled.tail), "tail"),
            ] {
                assert_gtf_eq(
                    global_transform(&disabled.app, off),
                    global_transform(&enabled.app, on),
                    &format!("frame {frame} {label} global"),
                );
                let enabled_tf = transform(&enabled.app, on);
                let disabled_tf = transform(&disabled.app, off);
                assert!(
                    quat_eq(disabled_tf.rotation, enabled_tf.rotation),
                    "frame {frame} {label} local rotation: {:?} vs {:?}",
                    disabled_tf.rotation,
                    enabled_tf.rotation
                );
            }
        }
    }

    /// Joints evaluate in the center node's space, so a joint's origin keeps its
    /// center-relative position while its rotation follows the center. The
    /// pre-spring chain publishes the center, so the opt-out cannot change the
    /// evaluation in either frame.
    #[test]
    fn with_center_space_post_spring_chain_gate_preserves_center_evaluation() {
        use crate::vrm::spring_bone::SpringRoot;

        let poses = [
            (Vec3::new(0.5, 0.25, -0.75), Quat::from_rotation_y(0.15)),
            (Vec3::new(-1.5, 1.0, 0.5), Quat::from_rotation_y(0.85)),
            (Vec3::new(2.0, -0.5, 1.25), Quat::from_rotation_y(1.4)),
        ];
        let mut enabled = center_space_spring_scene(false);
        let mut disabled = center_space_spring_scene(true);

        for scene in [&enabled, &disabled] {
            let root = scene.root;
            let center = scene
                .app
                .world()
                .get::<SpringRoot>(root)
                .unwrap()
                .center_node
                .0
                .expect("the center entity is present");
            assert_eq!(center, root);
            assert!(scene.app.world().get::<Transform>(center).is_some());
        }

        let mut previous_center: Option<GlobalTransform> = None;
        let mut joint_anchor: Option<Vec3> = None;
        for (frame, (translation, rotation)) in poses.into_iter().enumerate() {
            for scene in [&mut enabled, &mut disabled] {
                let mut tf = scene
                    .app
                    .world_mut()
                    .get_mut::<Transform>(scene.root)
                    .unwrap();
                tf.translation = translation;
                tf.rotation = rotation;
            }
            enabled.app.update();
            disabled.app.update();

            let center = global_transform(&enabled.app, enabled.root);
            assert_gtf_eq(
                global_transform(&disabled.app, disabled.root),
                center,
                &format!("frame {frame} center"),
            );
            if let Some(previous) = previous_center {
                assert!(
                    quat_angle(center.rotation(), previous.rotation()) > 0.1,
                    "the center must rotate between checkpoints: {center:?} vs {previous:?}"
                );
            }
            previous_center = Some(center);

            for ((on, off), label) in [
                ((enabled.joint, disabled.joint), "joint"),
                ((enabled.tail, disabled.tail), "tail"),
            ] {
                assert_gtf_eq(
                    global_transform(&disabled.app, off),
                    global_transform(&enabled.app, on),
                    &format!("frame {frame} {label} global"),
                );
                assert!(
                    quat_eq(
                        transform(&disabled.app, off).rotation,
                        transform(&enabled.app, on).rotation
                    ),
                    "frame {frame} {label} local rotation"
                );
                let disabled_relative = center_relative(&disabled.app, disabled.root, off);
                let enabled_relative = center_relative(&enabled.app, enabled.root, on);
                assert!(
                    enabled_relative.abs_diff_eq(disabled_relative, 1e-4),
                    "frame {frame} {label} center-relative position: {enabled_relative:?} vs {disabled_relative:?}"
                );
            }

            let anchor = center_relative(&enabled.app, enabled.root, enabled.joint);
            match joint_anchor {
                None => joint_anchor = Some(anchor),
                Some(expected) => assert!(
                    anchor.abs_diff_eq(expected, 1e-4),
                    "the joint origin keeps its center-relative position across frames: {anchor:?} vs {expected:?}"
                ),
            }
        }
    }

    /// `VrmPlugin` alone brings the `.vrm` loader; no other plugin is needed.
    #[test]
    fn vrm_plugin_adds_the_vrm_gltf_plugin() {
        let mut app = App::new();
        app.add_plugins((headless_default_plugins(), VrmPlugin));
        assert!(app.is_plugin_added::<VrmGltfPlugin>());
    }
}
