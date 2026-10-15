pub mod body_tracking;
pub mod components;
pub mod coords;
pub mod detach;
pub(crate) mod expressions;
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
use bevy::transform::systems::{propagate_parent_transforms, sync_simple_transforms};
use expressions::VrmExpressionPlugin;
use mtoon::MtoonMaterialPlugin;
use std::path::PathBuf;

pub mod prelude {
    pub use crate::vrm::{
        Initialized, RestGlobalTransform, RestTransform, RestWorldTransform, Vrm, VrmBone,
        VrmExpression, VrmPath, VrmPlugin,
        body_tracking::{BodyTracking, SmoothedGaze},
        components::{
            ConstraintExecutionOrder, LAYER_BOTH, LAYER_FIRST_PERSON_ONLY, LAYER_THIRD_PERSON_ONLY,
            PendingNodeConstraint, VrmConstraintKind, VrmHeadOnly, VrmLightLayersPlugin,
            VrmNodeConstraint, VrmNodeIndex, all_vrm_render_layers, both_view_mesh_layers,
            first_person_camera_layers, first_person_only_mesh_layers, third_person_camera_layers,
            third_person_only_mesh_layers, vrm_light_layers_for, widen_vrm_light_layers,
        },
        coords::{VrmForwardPolicy, resolve_forward_policy},
        detach::RequestDetachVrm,
        expressions::{
            ClearExpressions, ExpressionOverrideType, ModifyExpressions, SetExpressions,
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
///
/// It is inserted by the root-identity attach (see
/// [`crate::vrm::instance`]) once the scene instance is ready and its root data
/// has been bound onto the entity: the entity the application spawned with
/// [`VrmHandle`], or the child scene root of a raw
/// [`WorldAssetRoot`](bevy::world_serialization::WorldAssetRoot) spawn.
#[derive(Debug, Component, Reflect, Copy, Clone)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct Vrm;

impl Vrm {
    pub const ROOT_BONE: &'static str = "VRMC_vrm.root_bone";
}

/// The path to the VRM file.
/// This component is automatically inserted by the load-time glTF pipeline while
/// the `.vrm` file loads, from the loader's `LoadContext`.
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
/// [`RestGlobalTransform`] itself stays world-space because gaze and body
/// tracking consume it as a world-frame rotation.
#[derive(Debug, Copy, Clone, Component, Deref, Reflect, Default)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct RestWorldTransform(pub GlobalTransform);

marker_component!(
    /// A marker component attached to the entity of VRM, inserted **after** the
    /// rest of its root data, so `On<Insert, Initialized>` observes a fully
    /// populated root.
    ///
    /// The root-identity attach writes it exactly once per successful bind; a
    /// re-attach (model swap, hot reload) inserts it again, which is why the
    /// crate's own readiness hooks key on [`Insert`](bevy::ecs::lifecycle::Insert)
    /// rather than `Add`.
    Initialized
);

/// The main plugin for VRM support in Bevy.
///
/// This is the **runtime** half: it adds the systems that evaluate what
/// [`VrmGltfPlugin`](crate::prelude::VrmGltfPlugin) wrote into the scene while
/// the file loaded — spring bones, gaze control, expressions, node constraints
/// and `MToon` rendering — and the root-identity pipeline that binds that
/// load-time data onto the entity spawned with [`VrmHandle`].
///
/// It does not load a `.vrm` by itself. An app also needs
/// [`VrmGltfPlugin`](crate::prelude::VrmGltfPlugin), which is the only plugin
/// that claims the `vrm` extension; add `VrmaPlugin` on top for `.vrma`
/// playback.
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy_vrm1::prelude::*;
/// # let mut app = App::new();
/// app.add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin));
/// ```
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
