pub mod animation;
pub mod gltf;
mod loader;
pub mod retarget;
pub mod spawn;

use crate::vrma::animation::VrmaAnimationPlayersPlugin;
use crate::vrma::loader::{VrmaAsset, VrmaLoaderPlugin};
use crate::vrma::retarget::VrmaRetargetPlugin;
use crate::vrma::spawn::VrmaSpawnPlugin;
use bevy::app::App;
use bevy::asset::Handle;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

pub struct VrmaPlugin;

impl Plugin for VrmaPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<Vrma>()
            .register_type::<VrmaEntity>()
            .register_type::<VrmaHandle>()
            .register_type::<VrmaPath>()
            .register_type::<VrmaDuration>()
            .register_type::<RetargetTo>()
            .register_type::<RetargetSource>()
            .add_plugins((
                VrmaLoaderPlugin,
                VrmaSpawnPlugin,
                VrmaRetargetPlugin,
                VrmaAnimationPlayersPlugin,
            ));
    }
}

/// An asset handle to spawn VRMA.
#[derive(Debug, Component, Reflect)]
pub struct VrmaHandle(pub Handle<VrmaAsset>);

/// A marker component attached to the entity of VRMA.
#[derive(Debug, Component, Reflect, Copy, Clone, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct Vrma;

/// A new type pattern object to explicitly indicate the entity is VRMA.
#[derive(Debug, Reflect, Copy, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[reflect(Serialize, Deserialize)]
pub struct VrmaEntity(pub Entity);

/// Represents the path to the VRMA file.
///
/// This component is automatically attached to the entity with the same entity as [`VrmaHandle`] after loading VRMA.
#[derive(Component, Debug, Reflect, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[reflect(Component, Serialize, Deserialize)]
pub struct VrmaPath(pub PathBuf);

/// The component that holds the duration of VRMA's animation.
/// This component is automatically attached to the entity with the same entity as [`VrmaHandle`] after loading VRMA.
///
/// This component's structure will be changed in the future if VRMA can have multiple animations.
#[derive(Debug, Component, Reflect)]
pub struct VrmaDuration(pub Duration);

/// An event that is emitted when VRMA is loaded.
///
/// This event is emitted as a trigger.
/// The target of the trigger is the VRMA entity.
#[derive(Debug, Event, Copy, Clone)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub struct LoadedVrma {
    pub vrm: Entity,
}

/// The component that holds the entity to retarget.
/// This is used internally to retarget bones and expressions, and attached after vrma's entity children are spawned.
#[derive(Debug, Component, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
struct RetargetTo(pub Entity);

/// This is a component that indicates that it is the source of retargeting.
/// This is used internally to retarget bones and expressions, and attached after vrma's entity children are spawned.
#[derive(Component, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
struct RetargetSource;
