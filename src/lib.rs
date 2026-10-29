#![allow(clippy::type_complexity, clippy::too_many_arguments, unused)]

mod error;
mod macros;
pub mod system_param;
mod system_set;
pub mod vrm;
pub mod vrma;

pub mod prelude {
    pub use crate::{
        error::AppResult, system_param::prelude::*, system_set::VrmSystemSets, vrm::prelude::*,
        vrma::prelude::*,
    };
}

#[doc(hidden)]
#[cfg(test)]
pub(crate) mod tests {
    use crate::prelude::PlayVrma;
    use crate::vrm::instance::VrmInstanceState;
    use crate::vrm::runtime::apply_node_constraints;
    use crate::vrma::animation::animation_graph::RequestUpdateAnimationGraph;
    use bevy::MinimalPlugins;
    use bevy::animation::RepeatAnimation;
    use bevy::asset::AssetPlugin;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::gltf::Gltf;
    use bevy::prelude::*;
    use bevy::window::WindowPlugin;
    use bevy::world_serialization::WorldAsset;
    use std::time::Duration;

    pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    #[macro_export]
    macro_rules! success {
        () => {
            std::result::Result::Ok(())
        };
    }

    pub fn test_app() -> bevy::app::App {
        let mut app = bevy::app::App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            WindowPlugin::default(),
        ));
        app
    }

    pub fn run_frames(
        app: &mut App,
        count: usize,
    ) {
        for _ in 0..count {
            app.update();
        }
    }

    pub fn rotation(
        app: &App,
        entity: Entity,
    ) -> Quat {
        app.world().get::<Transform>(entity).unwrap().rotation
    }

    /// Rotates in place: the translation matters to the aim constraint, so it
    /// must survive.
    pub fn set_rotation(
        app: &mut App,
        entity: Entity,
        rotation: Quat,
    ) {
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Transform>()
            .expect("the entity has a Transform")
            .rotation = rotation;
    }

    /// `q` and `-q` are the same rotation, so compare up to sign.
    pub fn rotation_is(
        actual: Quat,
        expected: Quat,
    ) -> bool {
        actual.abs_diff_eq(expected, 1e-5) || actual.abs_diff_eq(-expected, 1e-5)
    }

    pub fn run_constraints(app: &mut App) {
        app.world_mut()
            .run_system_once(apply_node_constraints)
            .expect("Failed to run apply_node_constraints");
        app.world_mut().flush();
    }

    pub fn request_graph(
        app: &mut App,
        vrm: Entity,
    ) {
        app.world_mut().trigger(RequestUpdateAnimationGraph { vrm });
        app.world_mut().flush();
        app.update();
    }

    pub fn play_vrma(
        app: &mut App,
        vrma: Entity,
        transition_duration: Duration,
    ) {
        app.world_mut()
            .commands()
            .entity(vrma)
            .trigger(move |entity| PlayVrma {
                vrma: entity,
                repeat: RepeatAnimation::Never,
                transition_duration,
                reset_spring_bones: false,
            });
    }

    pub fn plain_scene() -> WorldAsset {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        world.spawn(ChildOf(root));
        WorldAsset::new(world)
    }

    pub fn gltf_with_scene(scene: Handle<WorldAsset>) -> Gltf {
        gltf_with_scenes(vec![scene])
    }

    pub fn gltf_with_scenes(scenes: Vec<Handle<WorldAsset>>) -> Gltf {
        Gltf {
            scenes,
            named_scenes: Default::default(),
            meshes: Vec::new(),
            named_meshes: Default::default(),
            materials: Vec::new(),
            named_materials: Default::default(),
            nodes: Vec::new(),
            named_nodes: Default::default(),
            skins: Vec::new(),
            named_skins: Default::default(),
            default_scene: None,
            animations: Vec::new(),
            named_animations: Default::default(),
            source: None,
        }
    }

    pub fn scene_root_of(
        app: &App,
        owner: Entity,
    ) -> Entity {
        app.world()
            .get::<VrmInstanceState>(owner)
            .expect("the owner records its attached scene root")
            .scene_root
    }
}
