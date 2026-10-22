//! Components the glTF extension handler writes into the scene `WorldAsset`
//! while a `.vrm` / `.vrma` file loads.
//!
//! Everything here is produced *inside* bevy's `GltfLoader`, before the scene
//! world is serialized into the asset, which imposes two rules on every type in
//! this module:
//!
//! * It must derive `Reflect` with `#[reflect(Component)]` and be handed to
//!   [`App::register_type`](bevy::app::App::register_type), because the scene
//!   spawn pipeline copies components out of the asset through reflection. An
//!   unregistered type fails the whole instantiation.
//! * Any component holding an [`Entity`] must mark that field `#[entities]`.
//!   Entity ids minted in the loader's scratch world mean nothing once the asset
//!   is instantiated, so an unremapped reference is not a harmless no-op but a
//!   dangling read into a different scene's entities.
//!
//! # Why `#[entities]` and not `#[derive(MapEntities)]`
//!
//! `#[derive(Component)]` turns every `#[entities]` field into a
//! `Component::map_entities`
//! implementation, and that is the hook bevy 0.19 calls when it instantiates a
//! `WorldAsset`:
//! `ReflectComponent::apply_or_insert_mapped` runs
//! `C::map_entities(&mut component, &mut mapper)` for every copied component
//! (`bevy_ecs/src/reflect/component.rs`), and it is
//! `world_asset.rs` that invokes it while spawning. The standalone
//! [`MapEntities`](bevy::ecs::entity::MapEntities) trait is a separate impl
//! that this path never consults — only `#[reflect(MapEntities)]` type data
//! would reach it — so deriving it would compile and then do nothing.
//!
//! Node identity is by glTF node index ([`VrmNodeIndex`]), never by
//! [`Name`]: glTF node names are optional and routinely collide inside one file.

use crate::macros::marker_component;
use bevy::prelude::*;

/// Marks a head-mesh copy split off by `VRMC_vrm.firstPerson`'s `auto`
/// classification: the face/eyes/hair geometry that must be hidden from a
/// first-person camera.
///
/// The copy is created at load time and lives in the scene asset, so this
/// marker survives every instantiation of it. Until
/// [`RequestEnableFirstPerson`](crate::prelude::RequestEnableFirstPerson), the
/// copy renders on the default layer next to the body half it was split from.
marker_component!(
    /// Marks a head-mesh copy split off by `firstPerson: auto`.
    VrmHeadOnly
);

/// The view `VRMC_vrm.firstPerson` restricts a mesh to, recorded at load time.
///
/// [`RequestEnableFirstPerson`](crate::prelude::RequestEnableFirstPerson) turns
/// it into render layers; a mesh without it is visible in both views.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) enum FirstPersonView {
    /// `thirdPersonOnly`, or the part of an `auto` mesh weighted to the head.
    ThirdPersonOnly,
    /// `firstPersonOnly`.
    FirstPersonOnly,
}

/// The glTF node index of the entity.
///
/// Nodes are addressed by index because glTF node names are optional and may
/// collide; every node spawned from a `.vrm` gets exactly one of these, and
/// lookups (`VRMC_vrm.humanoid`, `VRMC_node_constraint`, `VRMC_springBone`)
/// go through it.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct VrmNodeIndex(pub usize);

/// A `VRMC_node_constraint` read while a node was processed, before the whole
/// node index -> entity mapping is known.
///
/// `source_node` is a glTF node index; it is resolved into an [`Entity`] during
/// scene finalization, producing [`VrmNodeConstraint`]. This type exists only
/// inside the loader: nothing should see it at runtime.
#[derive(Component, Debug, Copy, Clone, PartialEq, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct PendingNodeConstraint {
    /// The glTF index of the source node (see [`VrmNodeIndex`]).
    pub source_node: usize,
    /// `VRMC_node_constraint.*.weight`, which defaults to `1.0`.
    pub weight: f32,
    /// The constraint variant, with its axis already parsed.
    pub kind: VrmConstraintKind,
}

/// The variant of a `VRMC_node_constraint`, with its axis parsed into a typed
/// [`Dir3`] at load time so no runtime code has to interpret the spec's axis
/// strings.
#[derive(Debug, Copy, Clone, PartialEq, Reflect)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub enum VrmConstraintKind {
    /// `VRMC_node_constraint.constraint.rotation`.
    Rotation,
    /// `VRMC_node_constraint.constraint.roll`.
    Roll {
        /// `rollAxis`: `X`, `Y` or `Z`. Expressed in the source node's rest space.
        roll_axis: Dir3,
    },
    /// `VRMC_node_constraint.constraint.aim`.
    Aim {
        /// `aimAxis`, one of `PositiveX` … `NegativeZ`. Expressed in the source
        /// node's rest space.
        aim_axis: Dir3,
    },
}

/// A resolved `VRMC_node_constraint`, living on the *destination* node.
#[derive(Component, Debug, Copy, Clone, PartialEq, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct VrmNodeConstraint {
    /// The source node's entity. Remapped on instantiation.
    #[entities]
    pub source: Entity,
    /// The blend weight; at `0.0` the destination is held at its rest rotation.
    pub weight: f32,
    /// The constraint variant.
    pub kind: VrmConstraintKind,
}

/// The constraints of one scene, topologically sorted by their dependencies.
///
/// The VRM spec requires constraints to be evaluated source-before-destination,
/// so the whole scene's constraint graph is sorted once at load time and the
/// resulting destination entities are listed here. Lives on the scene root;
/// every entry is remapped on instantiation.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Component, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct ConstraintExecutionOrder(#[entities] pub Vec<Entity>);

impl ConstraintExecutionOrder {
    /// The constraint destinations in evaluation order.
    #[must_use]
    pub fn destinations(&self) -> &[Entity] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::{ColliderShape, Head, HeadBoneEntity, Initialized, Vrm, VrmBone};
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::spring_bone::{SpringCenterNode, SpringColliders, SpringJoints, SpringRoot};
    use bevy::ecs::component::Component;
    use bevy::ecs::entity::EntityHashMap;
    use bevy::world_serialization::{
        DynamicWorld, WorldAsset, WorldInstanceSpawner, world_instance_spawner_system,
    };

    fn mapper(pairs: &[(Entity, Entity)]) -> EntityHashMap<Entity> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn constraint_execution_order_maps_its_entities() -> TestResult {
        let mut app = test_app();
        let first = app.world_mut().spawn_empty().id();
        let second = app.world_mut().spawn_empty().id();
        let mapped_first = app.world_mut().spawn_empty().id();
        let mapped_second = app.world_mut().spawn_empty().id();

        let mut order = ConstraintExecutionOrder(vec![first, second]);
        Component::map_entities(
            &mut order,
            &mut mapper(&[(first, mapped_first), (second, mapped_second)]),
        );

        assert_eq!(order.destinations(), [mapped_first, mapped_second]);
        success!()
    }

    #[test]
    fn node_constraint_maps_only_its_source_entity() -> TestResult {
        let mut app = test_app();
        let source = app.world_mut().spawn_empty().id();
        let mapped_source = app.world_mut().spawn_empty().id();

        let mut constraint = VrmNodeConstraint {
            source,
            weight: 0.5,
            kind: VrmConstraintKind::Aim {
                aim_axis: Dir3::NEG_Z,
            },
        };
        Component::map_entities(&mut constraint, &mut mapper(&[(source, mapped_source)]));

        assert_eq!(constraint.source, mapped_source);
        assert_eq!(constraint.weight, 0.5);
        assert_eq!(
            constraint.kind,
            VrmConstraintKind::Aim {
                aim_axis: Dir3::NEG_Z
            }
        );
        success!()
    }

    #[test]
    fn entities_outside_the_map_survive_untouched() -> TestResult {
        let mut app = test_app();
        let known = app.world_mut().spawn_empty().id();
        let unknown = app.world_mut().spawn_empty().id();
        let mapped_known = app.world_mut().spawn_empty().id();

        let mut order = ConstraintExecutionOrder(vec![known, unknown]);
        Component::map_entities(&mut order, &mut mapper(&[(known, mapped_known)]));

        assert_eq!(order.destinations(), [mapped_known, unknown]);
        success!()
    }

    /// The entity ids the loader's scratch world minted. Kept so a test can
    /// prove the copy *moved* a reference rather than agreeing by coincidence:
    /// both worlds mint ids from the same counter, so an equal id proves nothing.
    struct ScratchIds {
        root: Entity,
        head: Entity,
        joint: Entity,
        collider: Entity,
        center: Entity,
    }

    /// A scene shaped like the one `handler::scene` builds: a VRM root carrying
    /// `Vrm` + `Initialized`, one `<Bone>BoneEntity` holder on it, and a spring
    /// chain whose joint, collider and center node all name entities *of this
    /// world* — the only situation in which a stale reference is observable.
    ///
    /// The bones hang off the root through `ChildOf` because that is what
    /// `bevy_gltf` produces, and `set_instance_parent_sync` uses the absence of
    /// `ChildOf` to tell a scene root from its contents
    /// (`bevy_world_serialization/src/world_asset_spawner.rs`).
    fn pipeline_scene() -> (WorldAsset, ScratchIds) {
        let mut world = World::new();
        let head = world.spawn((VrmBone::from("head"), Head)).id();
        let joint = world.spawn(VrmBone::from("spring")).id();
        let collider = world.spawn(VrmBone::from("collider")).id();
        let center = world.spawn(VrmBone::from("center")).id();
        let root = world
            .spawn((
                Vrm,
                Initialized,
                HeadBoneEntity(head),
                SpringRoot {
                    joints: SpringJoints(vec![joint]),
                    colliders: SpringColliders(vec![(collider, ColliderShape::default())]),
                    center_node: SpringCenterNode(Some(center)),
                },
            ))
            .id();
        for bone in [head, joint, collider, center] {
            world.entity_mut(bone).insert(ChildOf(root));
        }
        (
            WorldAsset::new(world),
            ScratchIds {
                root,
                head,
                joint,
                collider,
                center,
            },
        )
    }

    /// A headless app that drives `WorldInstanceSpawner` by hand, so the scene
    /// goes through the production instantiation path rather than a stand-in.
    ///
    /// `WorldSerializationPlugin` is deliberately not used: its `Plugin` impl is
    /// `#[cfg(feature = "serialize")]`, which is not stable across this crate's
    /// builds. The three things it would do here are done explicitly instead —
    /// the same trade-off `crate::vrm::instance`'s tests make.
    fn spawner_app() -> App {
        let mut app = test_app();
        app.init_asset::<WorldAsset>()
            // `world_instance_spawner_system` reads `AssetEvent<DynamicWorld>`
            // unconditionally, so the spawner needs both asset storages.
            .init_asset::<DynamicWorld>()
            .init_resource::<WorldInstanceSpawner>()
            .register_type::<Vrm>()
            .register_type::<Initialized>()
            .register_type::<VrmBone>()
            .register_type::<Head>()
            .register_type::<HeadBoneEntity>()
            .register_type::<SpringRoot>()
            .register_type::<SpringJoints>()
            .register_type::<SpringColliders>()
            .register_type::<SpringCenterNode>()
            .register_type::<ColliderShape>()
            .register_type::<ChildOf>()
            .register_type::<Children>()
            .add_systems(Update, world_instance_spawner_system);
        app
    }

    /// The VRM root of the instance parented under `parent`.
    fn instance_root(
        app: &App,
        parent: Entity,
    ) -> Entity {
        let children = app
            .world()
            .entity(parent)
            .get::<Children>()
            .expect("the instance is parented under its spawn entity");
        assert_eq!(children.len(), 1, "exactly one VRM root per instance");
        children[0]
    }

    /// The definitive test: a scene asset instantiated the way the pipeline
    /// instantiates it, with every entity reference moved into the app world.
    ///
    /// A single world proves nothing here — the bug exists only across the copy
    /// — so two instances of one asset are spawned. Without a remap both roots
    /// name the *same* scratch-world ids, so neither instance's head exists in
    /// the app world and the two holders cannot disagree. With the remap each
    /// instance owns a distinct set of entities, every holder and every spring
    /// reference resolves inside its own instance, and nothing names an id the
    /// loader minted.
    #[test]
    fn scene_instantiation_remaps_bone_and_spring_references() -> TestResult {
        let mut app = spawner_app();
        let (scene, scratch) = pipeline_scene();
        let scene = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(scene);
        let first_parent = app.world_mut().spawn_empty().id();
        let second_parent = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_scope(|_world, mut spawner: Mut<WorldInstanceSpawner>| {
                spawner.spawn_as_child(scene.clone(), first_parent);
                spawner.spawn_as_child(scene.clone(), second_parent);
            });
        // This harness runs the spawner on `Update`; allow its deferred ready
        // event to be delivered before inspecting the instances.
        for _ in 0..2 {
            app.update();
        }

        let first_root = instance_root(&app, first_parent);
        let second_root = instance_root(&app, second_parent);
        assert_ne!(
            first_root, second_root,
            "two instances are two sets of entities"
        );
        assert_ne!(
            first_root, scratch.root,
            "the VRM root itself is a fresh entity of the app world"
        );

        // The `<Bone>BoneEntity` holder: gaze control, body tracking and the
        // first-person auto split all read it off the root.
        let mut heads = Vec::new();
        for root in [first_root, second_root] {
            heads.push(
                app.world()
                    .get::<HeadBoneEntity>(root)
                    .expect("the holder survived the copy")
                    .0,
            );
        }
        assert_ne!(
            heads[0], heads[1],
            "each instance must address its own head, not a shared id"
        );
        for head in &heads {
            assert_ne!(
                *head, scratch.head,
                "the holder still names the loader's scratch world"
            );
            assert_eq!(
                app.world().get::<VrmBone>(*head),
                Some(&VrmBone::from("head")),
                "the remapped holder must resolve to this instance's head"
            );
            assert!(
                app.world().get::<Head>(*head).is_some(),
                "the referenced entity exists in the app world, with its bone marker"
            );
        }

        // The spring chain: joints, colliders and the center node are all
        // references into the same world, and a chain that verlet-integrates
        // against the scratch world shares no transform with the avatar.
        for root in [first_root, second_root] {
            let spring = app
                .world()
                .get::<SpringRoot>(root)
                .expect("the chain's spring root survived the copy");
            let joint = spring.joints.first().copied().expect("one joint");
            let (collider, shape) = *spring.colliders.first().expect("one collider");
            let center = spring.center_node.0.expect("the chain declared a center");

            assert_ne!(
                joint, scratch.joint,
                "a joint still names the scratch world"
            );
            assert_ne!(
                collider, scratch.collider,
                "a collider still names the scratch world"
            );
            assert_ne!(
                center, scratch.center,
                "the center node still names the scratch world"
            );
            assert_eq!(
                app.world().get::<VrmBone>(joint),
                Some(&VrmBone::from("spring"))
            );
            assert_eq!(
                app.world().get::<VrmBone>(collider),
                Some(&VrmBone::from("collider"))
            );
            assert_eq!(
                app.world().get::<VrmBone>(center),
                Some(&VrmBone::from("center"))
            );
            // Only the entity half of a `(Entity, ColliderShape)` pair is a
            // reference; the shape is data and must survive the copy untouched.
            assert_eq!(shape, ColliderShape::default());
        }
        success!()
    }

    /// The same remap, one level down: the component's own `map_entities`
    /// implementation. `scene_instantiation_remaps_bone_and_spring_references`
    /// is the end-to-end proof; this pins which of the three holders does the
    /// work, so a regression names its own type.
    #[test]
    fn spring_root_and_a_bone_holder_map_their_own_references() -> TestResult {
        let mut app = test_app();
        let head = app.world_mut().spawn_empty().id();
        let joint = app.world_mut().spawn_empty().id();
        let collider = app.world_mut().spawn_empty().id();
        let center = app.world_mut().spawn_empty().id();
        let (mapped_head, mapped_joint, mapped_collider, mapped_center) = (
            app.world_mut().spawn_empty().id(),
            app.world_mut().spawn_empty().id(),
            app.world_mut().spawn_empty().id(),
            app.world_mut().spawn_empty().id(),
        );
        let mut map = mapper(&[
            (head, mapped_head),
            (joint, mapped_joint),
            (collider, mapped_collider),
            (center, mapped_center),
        ]);

        let mut holder = HeadBoneEntity(head);
        Component::map_entities(&mut holder, &mut map.clone());
        assert_eq!(holder.0, mapped_head);

        let mut spring = SpringRoot {
            joints: SpringJoints(vec![joint]),
            colliders: SpringColliders(vec![(collider, ColliderShape::default())]),
            center_node: SpringCenterNode(Some(center)),
        };
        Component::map_entities(&mut spring, &mut map);
        assert_eq!(*spring.joints, [mapped_joint]);
        assert_eq!(spring.colliders[0].0, mapped_collider);
        assert_eq!(spring.colliders[0].1, ColliderShape::default());
        assert_eq!(spring.center_node.0, Some(mapped_center));
        success!()
    }
}
