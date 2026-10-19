pub(crate) mod animation;
mod gltf;
mod initialize;
mod loader;

#[cfg(test)]
mod integration_tests;

use crate::macros::{entity_component, marker_component};
use crate::vrma::animation::VrmaAnimationPlayersPlugin;
use crate::vrma::initialize::VrmaInitializePlugin;
use crate::vrma::loader::{VrmaAsset, VrmaLoaderPlugin};
use bevy::animation::ActiveAnimation;
use bevy::animation::graph::AnimationGraph;
use bevy::app::App;
use bevy::asset::Handle;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use std::path::PathBuf;
use std::time::Duration;

pub mod prelude {
    pub use crate::vrm::expressions::ExpressionCategory;
    pub use crate::vrma::{
        LoadedVrma, Vrma, VrmaDuration, VrmaEntity, VrmaHandle, VrmaPath, VrmaPlugin,
        animation::prelude::*, loader::VrmaAsset,
    };
}

/// This plugin enables support for VRMA (VRM Animation) in Bevy.
/// Please refer to [`VrmaHandle`] for details.
pub struct VrmaPlugin;

impl Plugin for VrmaPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_plugins((
            VrmaLoaderPlugin,
            VrmaInitializePlugin,
            VrmaAnimationPlayersPlugin,
        ));

        app.register_type::<Vrma>()
            .register_type::<VrmaEntity>()
            .register_type::<VrmaHandle>()
            .register_type::<VrmaPath>()
            .register_type::<VrmaDuration>()
            .register_type::<RetargetSource>()
            .register_type::<VrmAnimationClipHandle>()
            .register_type::<VrmAnimationNodeIndex>()
            .register_type::<VrmAnimationSourceClip>()
            .register_type::<VrmaRetargetedFor>()
            .register_type::<VrmaRetargetPending>()
            .register_type::<VrmaLoaded>()
            .register_type::<VrmaNeedsRestore>()
            .register_type::<VrmAnimationGraphState>()
            .register_type::<VrmaPlaybackState>();
    }
}

/// An asset handle to spawn VRMA.
///
/// After this handle is loaded, the following components are inserted. Then this handle is removed.
///
/// - [`Vrma`]
/// - [`VrmaPath`]
/// - [`VrmaDuration`]
/// - [`BoneRestTransform`](crate::prelude::RestTransform)
/// - [`BoneRestGlobalTransform`](crate::prelude::RestGlobalTransform)
/// - [`WorldAssetRoot`](bevy::world_serialization::WorldAssetRoot)
/// - [`VrmaAnimationPlayers`](crate::prelude::VrmaAnimationPlayers)
/// - Components hold the entity of each bone, refer to [here](crate::vrm::humanoid_bone) for more details.
#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
pub struct VrmaHandle(pub Handle<VrmaAsset>);

marker_component!(
    /// A marker component attached to the entity of VRMA.
    /// This component is automatically inserted after the [`VrmaHandle`](crate::vrma::VrmaHandle) is loaded.
    Vrma
);

entity_component!(
    /// Represents the entity of VRMA.
    ///
    /// This is used to retarget bones and expressions.
    VrmaEntity
);

/// Represents the path to the VRMA file.
///
/// This component is automatically attached to the entity with the same entity as [`VrmaHandle`] after loading VRMA.
#[derive(Component, Debug, Clone, Eq, PartialEq, Reflect, Deref)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct VrmaPath(pub PathBuf);
/// The component that holds the duration of VRMA's animation.
/// This component is automatically attached to the entity with the same entity as [`VrmaHandle`] after loading VRMA.
///
/// This component's structure will be changed in the future if VRMA can have multiple animations.
#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct VrmaDuration(pub Duration);

/// An event that is emitted when VRMA is loaded.
///
/// Emitted once the model's pending VRMA children have initialized and the
/// animation graph has assigned this clip a node index, so observers can play it.
/// It is emitted once per VRMA entity; a rebuilt rig restores the playback instead.
///
/// This event is emitted as a trigger.
/// The target of the trigger is the VRMA entity.
#[derive(Debug, EntityEvent, Copy, Clone, Reflect)]
pub struct LoadedVrma {
    pub vrm: Entity,
    #[event_target]
    pub vrma: Entity,
}

/// The component that holds the animation clip of VRMA.
#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
pub(crate) struct VrmAnimationClipHandle(pub Handle<AnimationClip>);

/// The component that holds the animation node index for VRMA.
#[derive(Debug, Component, Reflect, Copy, Clone, Default)]
#[reflect(Component, Default)]
pub(crate) struct VrmAnimationNodeIndex(pub AnimationNodeIndex);

/// The pristine animation clip of VRMA, re-cloned when the rig is replaced.
#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
pub(crate) struct VrmAnimationSourceClip(pub Handle<AnimationClip>);

/// The rig generation and the owner VRM the working clip was retargeted for.
#[derive(Debug, Component, Reflect, Copy, Clone, PartialEq, Eq)]
#[reflect(Component)]
pub(crate) struct VrmaRetargetedFor(pub u64, pub Entity);

/// Marker set on a VRMA once its first [`LoadedVrma`] has been emitted.
#[derive(Debug, Component, Reflect, Default)]
#[reflect(Component)]
pub(crate) struct VrmaLoaded;

/// Marker: the recorded playback must be re-applied to the rebuilt player.
#[derive(Debug, Component, Reflect, Default)]
#[reflect(Component)]
pub(crate) struct VrmaNeedsRestore;

/// Marker: the working clip has not been retargeted for the current rig yet.
#[derive(Debug, Component, Reflect, Default)]
#[reflect(Component)]
pub(crate) struct VrmaRetargetPending;

/// The per-VRM animation graph, reused across rig rebuilds so node indices stay valid.
#[derive(Component, Reflect, Clone)]
#[reflect(Component)]
pub(crate) struct VrmAnimationGraphState {
    pub graph: Handle<AnimationGraph>,
    /// The root bone's transitions, restored onto a rebuilt root bone.
    pub transitions: AnimationTransitions,
    pub root_bone: Entity,
    /// The graph node owned by each VRMA entity.
    pub(crate) nodes: HashMap<Entity, AnimationNodeIndex>,
}

impl Default for VrmAnimationGraphState {
    fn default() -> Self {
        Self {
            graph: Handle::default(),
            transitions: AnimationTransitions::default(),
            root_bone: Entity::PLACEHOLDER,
            nodes: HashMap::default(),
        }
    }
}

/// A snapshot of a VRMA's live playback, restored onto a rebuilt rig.
#[derive(Debug, Component, Reflect, Clone, Default)]
#[reflect(Component)]
pub(crate) struct VrmaPlaybackState {
    /// Whether the clip has been started and not stopped.
    pub active: bool,
    /// The live [`ActiveAnimation`] last observed.
    pub animation: Option<ActiveAnimation>,
}

/// This is a component that indicates that it is the source of retargeting.
/// This is used internally to retarget bones and expressions, and attached after vrma's entity children are spawned.
#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) struct RetargetSource;
