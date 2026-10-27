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
        Initialized, RestGlobalTransform, RestTransform, RestWorldTransform, Vrm, VrmBone,
        VrmExpression, VrmPath, VrmPlugin,
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
        look_at::LookAt,
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
                .after(VrmSystemSets::SpringBone),
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::PluginGroupBuilder;
    use bevy::render::RenderPlugin;
    use bevy::render::settings::WgpuSettings;

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

    /// `VrmPlugin` alone brings the `.vrm` loader; no other plugin is needed.
    #[test]
    fn vrm_plugin_adds_the_vrm_gltf_plugin() {
        let mut app = App::new();
        app.add_plugins((headless_default_plugins(), VrmPlugin));
        assert!(app.is_plugin_added::<VrmGltfPlugin>());
    }
}
