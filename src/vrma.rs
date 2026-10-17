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

/// The rig generation the working clip's curves were retargeted and baked for,
/// together with the avatar that owns it.
///
/// Absent means "never retargeted". A value different from the current
/// generation *or* owner tells the graph build to clone
/// [`VrmAnimationSourceClip`] again before retargeting. The owner is part of the
/// stamp because a generation counter alone aliases: a `.vrma` reparented under
/// a second avatar that also happens to be generation `1` would otherwise look
/// already retargeted, and its working clip would stay baked for the previous
/// rig's rest pose. A hand-built VRM without `VrmGeneration` is generation `0`.
#[derive(Debug, Component, Reflect, Copy, Clone, PartialEq, Eq)]
#[reflect(Component)]
pub(crate) struct VrmaRetargetedFor(pub u64, pub Entity);

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

/// A VRMA whose working clip has not yet been retargeted for the current rig.
///
/// Set by the graph build when it (re)clones the working clip, cleared by the
/// retarget once the curves are moved. The restore is gated on its absence, so
/// an unretargeted (source-space) clip is never played, and a retry asks for the
/// build again once the rig is reachable.
#[derive(Debug, Component, Reflect, Default)]
#[reflect(Component)]
pub(crate) struct VrmaRetargetPending;

/// The per-avatar animation graph and the playback bookkeeping that must outlive
/// a rig.
///
/// Stored on the stable VRM root, never on the root bone: the graph asset is
/// reused across rebuilds so every [`VrmAnimationNodeIndex`] stays valid (node
/// indices are stable for the asset's whole life and are never remapped), and
/// the [`AnimationTransitions`] snapshot lets a future
/// [`PlayVrma`](crate::prelude::PlayVrma) fade out the restored main clip
/// instead of leaving it stuck at full weight.
///
/// Stability does **not** mean a retired clip is retained: a `.vrma` that left
/// the avatar has its node made inert and its clip handle released, the node is
/// only kept so ids never shift.
#[derive(Component, Reflect, Clone)]
#[reflect(Component)]
pub(crate) struct VrmAnimationGraphState {
    /// The one graph asset this avatar plays from. A rebuild mutates this asset
    /// in place (updating clip handles and appending nodes), it never replaces
    /// it.
    pub graph: Handle<AnimationGraph>,
    /// The live [`AnimationTransitions`] of the current root bone, snapshotted
    /// every frame and restored onto a freshly attached root bone.
    pub transitions: AnimationTransitions,
    /// The root bone the snapshot was taken from, used to tell a fresh rig from
    /// a same-rig rebuild.
    pub root_bone: Entity,
    /// Which graph node each `.vrma` owning a node here owns, keyed by that
    /// child's entity.
    ///
    /// Ownership cannot be read off the `.vrma`'s own
    /// [`VrmAnimationNodeIndex`]: a `.vrma` reparented under another avatar
    /// still carries the id it owned elsewhere, and reusing that id in the new
    /// graph would overwrite a clip the new avatar owns. Membership is tracked
    /// here, per entity, so a node is only ever reused by the child that created
    /// it. Entries are kept for retired children so a re-added `.vrma` gets its
    /// own node back rather than a shifted one.
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

/// A private snapshot of a VRMA's live playback.
///
/// The player lives on the rig and dies with it, so what it was doing is
/// captured while it still exists and re-applied after a replacement. `active`
/// is authoritative — written by [`PlayVrma`](crate::prelude::PlayVrma) and
/// [`StopVrma`](crate::prelude::StopVrma) — while `animation` is refreshed from
/// the live player every frame the rig exists.
#[derive(Debug, Component, Reflect, Clone, Default)]
#[reflect(Component)]
pub(crate) struct VrmaPlaybackState {
    /// Whether the clip has been started and not stopped.
    ///
    /// Kept apart from `animation` because a clip can be active before the rig's
    /// player has ever been observed (a `PlayVrma` that arrived in the same
    /// frame as a replacement), and because a stopped clip has no live
    /// `ActiveAnimation` to read.
    pub active: bool,
    /// A verbatim copy of the live [`ActiveAnimation`] last observed.
    ///
    /// `ActiveAnimation` is `Copy`, so this holds every field Bevy stores —
    /// including the private `completions`, `elapsed`, `just_completed` and
    /// `last_seek_time` — and the restore writes it back with a single
    /// `*live = snapshot`, so nothing restarts or resets.
    pub animation: Option<ActiveAnimation>,
}

/// This is a component that indicates that it is the source of retargeting.
/// This is used internally to retarget bones and expressions, and attached after vrma's entity children are spawned.
#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) struct RetargetSource;
