pub(crate) mod animation;
mod gltf;
mod initialize;
mod loader;

use crate::macros::{entity_component, marker_component};
use crate::vrma::animation::VrmaAnimationPlayersPlugin;
use crate::vrma::initialize::VrmaInitializePlugin;
use crate::vrma::loader::{VrmaAsset, VrmaLoaderPlugin};
use bevy::animation::RepeatAnimation;
use bevy::animation::graph::AnimationMask;
use bevy::app::App;
use bevy::asset::Handle;
use bevy::prelude::*;
use std::path::PathBuf;
use std::time::Duration;

pub mod prelude {
    // `ExpressionCategory` is a `vrm` concept that `vrm::prelude` does not
    // export yet; it is re-exported here because
    // [`ExpressionSetting`](crate::vrma::animation::properties::ExpressionSetting)
    // is public API and stores one per expression.
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
            .register_type::<VrmaLoaded>()
            .register_type::<VrmaNeedsRestore>()
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

/// An event that is emitted the first time a `.vrma` is playable.
///
/// Emitted once the model's pending VRMA children have initialized, the
/// animation graph has assigned this clip a node index and the retarget has
/// been baked into a playable clip, so observers can play it.
///
/// It is emitted **once per VRMA entity**. When a rig is replaced, the VRMA
/// keeps playing: the engine restores the recorded playback onto the new player
/// instead of re-emitting this event, so an observer that starts the clip does
/// not fight the restore.
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

/// The pristine, never-retargeted animation clip a `.vrma` was loaded from.
///
/// [`VrmAnimationClipHandle`] is a per-instance clone the retarget and bake
/// mutate in place, so it cannot be retargeted a second time. Replacing a rig
/// has to start over from an untouched clip, and that is what this strong
/// handle keeps alive for the VRMA entity's whole life.
#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
pub(crate) struct VrmAnimationSourceClip(pub Handle<AnimationClip>);

/// The rig generation the working clip's curves were retargeted and baked for.
///
/// Absent means "never retargeted". A value different from the avatar's current
/// generation (see `crate::vrm::instance::VrmGeneration`) tells the graph build
/// to clone [`VrmAnimationSourceClip`] again before retargeting. A hand-built
/// VRM without that component is generation `0`.
#[derive(Debug, Component, Reflect, Copy, Clone, Default, PartialEq, Eq)]
#[reflect(Component)]
pub(crate) struct VrmaRetargetedFor(pub u64);

/// Marker set on a VRMA once its first [`LoadedVrma`] has been emitted.
///
/// A reattachment restores the recorded playback instead of signalling a fresh
/// load, so app observers do not restart the clip.
#[derive(Debug, Component, Reflect, Default)]
#[reflect(Component)]
pub(crate) struct VrmaLoaded;

/// A VRMA whose graph binding was just rebuilt and whose live playback must be
/// re-applied to the new player.
#[derive(Debug, Component, Reflect, Default)]
#[reflect(Component)]
pub(crate) struct VrmaNeedsRestore;

/// A private snapshot of a VRMA's live playback and graph mask.
///
/// The player and graph live on the rig and die with it, so what they were
/// doing is captured while they still exist and re-applied after a replacement.
/// `active` is authoritative — written by [`PlayVrma`](crate::prelude::PlayVrma)
/// and [`StopVrma`](crate::prelude::StopVrma) — while the remaining fields are
/// refreshed from the live animation every frame.
#[derive(Debug, Component, Reflect, Clone)]
#[reflect(Component)]
pub(crate) struct VrmaPlaybackState {
    /// Whether the clip has been started and not stopped.
    pub active: bool,
    /// Whether the live animation was paused.
    pub paused: bool,
    /// Playback position in seconds.
    pub seek_time: f32,
    /// Playback speed multiplier.
    pub speed: f32,
    /// Repetition mode.
    pub repeat: RepeatAnimation,
    /// Active animation weight.
    pub weight: f32,
    /// The graph node's mask-out bitfield (`0` means "nothing masked out"),
    /// as last observed.
    pub mask: AnimationMask,
}

impl Default for VrmaPlaybackState {
    fn default() -> Self {
        Self {
            active: false,
            paused: false,
            seek_time: 0.0,
            speed: 1.0,
            repeat: RepeatAnimation::Never,
            weight: 1.0,
            mask: 0,
        }
    }
}

/// This is a component that indicates that it is the source of retargeting.
/// This is used internally to retarget bones and expressions, and attached after vrma's entity children are spawned.
#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) struct RetargetSource;
