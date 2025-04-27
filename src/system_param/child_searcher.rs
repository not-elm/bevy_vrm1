use crate::vrm::humanoid_bone::HumanoidBoneRegistry;
use crate::vrm::VrmBone;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

#[derive(SystemParam)]
pub struct ChildSearcher<'w, 's> {
    entities: Query<
        'w,
        's,
        (
            Option<&'static Name>,
            Option<&'static VrmBone>,
            Option<&'static Children>,
        ),
    >,
}

impl ChildSearcher<'_, '_> {
    pub fn has_been_spawned_all_bones(
        &self,
        root: Entity,
        bone_registry: &HumanoidBoneRegistry,
    ) -> bool {
        bone_registry
            .values()
            .all(|bone_name| self.find_from_name(root, bone_name.as_str()).is_some())
    }

    pub fn find_from_name(
        &self,
        root: Entity,
        target_name: &str,
    ) -> Option<Entity> {
        find_entity(target_name, false, root, &self.entities)
    }

    pub fn find_from_bone_name(
        &self,
        root: Entity,
        target_name: &VrmBone,
    ) -> Option<Entity> {
        find_entity(target_name, true, root, &self.entities)
    }
}

fn find_entity(
    target_name: &str,
    is_bone: bool,
    entity: Entity,
    entities: &Query<(Option<&Name>, Option<&VrmBone>, Option<&Children>)>,
) -> Option<Entity> {
    let (name, bone, children) = entities.get(entity).ok()?;
    if is_bone {
        if bone.is_some_and(|bone| bone.0 == target_name) {
            return Some(entity);
        }
    } else if name.is_some_and(|name| name.as_str() == target_name) {
        return Some(entity);
    }

    for child in children? {
        if let Some(entity) = find_entity(target_name, is_bone, *child, entities) {
            return Some(entity);
        }
    }
    None
}
