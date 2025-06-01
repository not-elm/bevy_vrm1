pub mod play;
mod setup;

use crate::vrma::animation::play::VrmaAnimationPlayPlugin;
use crate::vrma::animation::setup::VrmaAnimationSetupPlugin;
use bevy::app::App;
use bevy::asset::Handle;
use bevy::prelude::*;

#[cfg(feature = "reflect")]
use serde::{Deserialize, Serialize};

pub struct VrmaAnimationPlayersPlugin;

impl Plugin for VrmaAnimationPlayersPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_plugins((VrmaAnimationSetupPlugin, VrmaAnimationPlayPlugin));
        #[cfg(feature = "reflect")]
        {
            app.register_type::<AnimationPlayerEntityTo>()
                .register_type::<VrmAnimationGraph>();
        }
    }
}

/// After spawn the vrma, the animation player will be spawned.
/// This component is used to hold that entity in the root entity.
#[derive(Component, Debug, Deref, DerefMut)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
pub struct AnimationPlayerEntityTo(pub Entity);

#[derive(Component, Default)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub struct VrmAnimationGraph {
    pub handle: Handle<AnimationGraph>,
    pub nodes: Vec<AnimationNodeIndex>,
}

impl VrmAnimationGraph {
    pub fn new(
        clip: impl IntoIterator<Item = Handle<AnimationClip>>,
        animation_graphs: &mut Assets<AnimationGraph>,
    ) -> Self {
        let (graph, nodes) = AnimationGraph::from_clips(clip);
        let handle = animation_graphs.add(graph);

        Self { handle, nodes }
    }
}
