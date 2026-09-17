pub(crate) mod animation;
mod gltf;
mod initialize;
mod loader;

use crate::macros::{entity_component, marker_component};
use crate::vrma::animation::VrmaAnimationPlayersPlugin;
use crate::vrma::initialize::VrmaInitializePlugin;
use crate::vrma::loader::{VrmaAsset, VrmaLoaderPlugin};
use bevy::app::App;
use bevy::asset::Handle;
use bevy::prelude::*;
use std::path::PathBuf;
use std::time::Duration;

pub mod prelude {
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
            .register_type::<VrmAnimationNodeIndex>();
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

/// This is a component that indicates that it is the source of retargeting.
/// This is used internally to retarget bones and expressions, and attached after vrma's entity children are spawned.
#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) struct RetargetSource;
