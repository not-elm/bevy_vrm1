use crate::vrm::Vrm;
use crate::vrm::VrmBone;
use crate::vrm::humanoid_bone::HumanoidBoneRegistry;
use crate::vrm::instance::{VrmHandle, VrmInstanceState};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

#[derive(SystemParam)]
pub struct ChildSearcher<'w, 's> {
    managed_roots: Query<'w, 's, (Option<&'static VrmInstanceState>, Has<VrmHandle>)>,
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
    /// A managed avatar searches only its current scene, excluding user-owned
    /// siblings such as VRMA source rigs and nested avatars. An unattached
    /// managed avatar has no searchable rig yet.
    fn search_root(
        &self,
        entity: Entity,
    ) -> Option<Entity> {
        let (instance, managed) = self.managed_roots.get(entity).ok()?;
        match instance {
            Some(instance) => Some(instance.scene_root),
            None if managed => None,
            None => Some(entity),
        }
    }

    #[inline]
    pub fn find_root_bone(
        &self,
        vrm: Entity,
    ) -> Option<Entity> {
        self.find_from_name(vrm, Vrm::ROOT_BONE)
    }

    pub fn find_from_name(
        &self,
        root: Entity,
        target_name: &str,
    ) -> Option<Entity> {
        find_entity(target_name, false, self.search_root(root)?, &self.entities)
    }

    pub fn find_by_bone_name(
        &self,
        root: Entity,
        target_name: &VrmBone,
    ) -> Option<Entity> {
        find_entity(target_name, true, self.search_root(root)?, &self.entities)
    }

    /// Whether every bone of `bone_registry` has a spawned entity named after it
    /// under `root`.
    ///
    /// A `.vrm` writes its bones into the scene asset, so the loader knows them
    /// up front; a `.vrma` builds a [`HumanoidBoneRegistry`] from its own glTF
    /// node names and its source rig arrives with the instantiated scene, so the
    /// VRMA side has to wait for this before it can be marked initialized.
    pub(crate) fn has_been_spawned_all_bones(
        &self,
        root: Entity,
        bone_registry: &HumanoidBoneRegistry,
    ) -> bool {
        bone_registry
            .values()
            .all(|bone_name| self.find_from_name(root, bone_name.as_str()).is_some())
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

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::tests::test_app;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use bevy_test_helper::system::SystemExt;

    #[test]
    fn test_find_root_bone() {
        let mut app = test_app();

        let vrm = app.world_mut().spawn_empty().id();
        app.world_mut()
            .commands()
            .entity(vrm)
            .with_child(Name::new(Vrm::ROOT_BONE));
        app.update();

        app.world_mut()
            .run_system_once(move |s: ChildSearcher| s.find_root_bone(vrm))
            .unwrap()
            .expect("Failed to find root bone");
    }

    #[test]
    fn a_managed_avatar_searches_its_scene_not_a_nested_avatar() {
        let mut app = test_app();
        let owner = app.world_mut().spawn(VrmHandle(Handle::default())).id();
        let nested = app.world_mut().spawn(ChildOf(owner)).id();
        app.world_mut()
            .spawn((Name::new(Vrm::ROOT_BONE), ChildOf(nested)));
        app.world_mut()
            .spawn((Name::new("head"), VrmBone::from("head"), ChildOf(nested)));
        let scene = app.world_mut().spawn(ChildOf(owner)).id();
        let root_bone = app
            .world_mut()
            .spawn((Name::new(Vrm::ROOT_BONE), ChildOf(scene)))
            .id();
        let head = app
            .world_mut()
            .spawn((Name::new("head"), VrmBone::from("head"), ChildOf(root_bone)))
            .id();
        app.world_mut()
            .entity_mut(owner)
            .insert(crate::vrm::instance::VrmInstanceState { scene_root: scene });

        let found = app
            .world_mut()
            .run_system_once(move |searcher: ChildSearcher| {
                (
                    searcher.find_root_bone(owner),
                    searcher.find_from_name(owner, "head"),
                    searcher.find_by_bone_name(owner, &VrmBone::from("head")),
                )
            })
            .unwrap();
        assert_eq!(found, (Some(root_bone), Some(head), Some(head)));
    }

    #[test]
    fn an_unattached_managed_avatar_does_not_borrow_a_child_rig() {
        let mut app = test_app();
        let owner = app.world_mut().spawn(VrmHandle(Handle::default())).id();
        app.world_mut()
            .spawn((Name::new(Vrm::ROOT_BONE), ChildOf(owner)));
        let found = app
            .world_mut()
            .run_system_once(move |searcher: ChildSearcher| searcher.find_root_bone(owner))
            .unwrap();
        assert_eq!(found, None);
    }
}
