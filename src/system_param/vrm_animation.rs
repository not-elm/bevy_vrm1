use crate::prelude::ChildSearcher;
use bevy::ecs::system::SystemParam;
use bevy::prelude::{AnimationPlayer, Entity, Query, Reflect};

#[derive(SystemParam)]
pub struct VrmAnimation<'w, 's> {
    searcher: ChildSearcher<'w, 's>,
    players: Query<'w, 's, &'static AnimationPlayer>,
}

impl VrmAnimation<'_, '_> {
    pub fn all_finished(
        &self,
        vrm: Entity,
    ) -> bool {
        if let Some(root_bone) = self.searcher.find_root_bone(vrm)
            && let Ok(animation_player) = self.players.get(root_bone)
            && !animation_player.all_finished()
        {
            return false;
        }
        self.finished_expressions(vrm)
    }

    pub fn finished_humanoid_bones(
        &self,
        vrm: Entity,
    ) -> bool {
        if let Some(root_bone) = self.searcher.find_root_bone(vrm)
            && let Ok(animation_player) = self.players.get(root_bone)
            && !animation_player.all_finished()
        {
            false
        } else {
            true
        }
    }

    pub fn finished_expressions(
        &self,
        vrm: Entity,
    ) -> bool {
        self.finished_humanoid_bones(vrm)
    }
}
