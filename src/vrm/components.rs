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
//!   unregistered type is dropped, not copied.
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
//! (`bevy_ecs/src/reflect/component.rs:340`, `:345`, `:353`), and it is
//! `world_asset.rs:192` that invokes it while spawning. The standalone
//! [`MapEntities`](bevy::ecs::entity::MapEntities) trait is a separate impl
//! that this path never consults — only `#[reflect(MapEntities)]` type data
//! would reach it — so deriving it would compile and then do nothing.
//!
//! Node identity is by glTF node index ([`VrmNodeIndex`]), never by
//! [`Name`]: glTF node names are optional and routinely collide inside one file.
//!
//! # First/third-person render layers
//!
//! The helpers at the bottom of this module split the two camera views while
//! both cameras still render ordinary layer-0 scene content.
//!
//! The scheme is forced by bevy's semantics: visibility is a plain bitwise AND
//! over the layer set (`RenderLayers::intersects`, used at
//! `bevy_camera/src/visibility/mod.rs:809` and
//! `bevy_pbr/src/render/mesh.rs:2294`). There is no exclusion, so **two sets
//! separate only if they share no layer at all**. Putting [`LAYER_BOTH`] on a
//! "…only" mesh set therefore makes that set intersect *every* camera that also
//! carries [`LAYER_BOTH`] — which is exactly what an ordinary scene camera has —
//! and the separation collapses. The exclusive mesh sets must therefore carry
//! their layer alone, which is why they are `{1}` and `{2}` and not `{0, 1}` /
//! `{0, 2}`. `render_layer_helpers_pin_the_visibility_matrix` pins the matrix.
//!
//! # The light obligation, and how it is discharged
//!
//! A mesh on layer 1 or 2 alone is *directly lit* by a layer-0 light as usual:
//! bevy 0.19.1 does not filter direct lighting by mesh layers. A light is
//! gathered per **view** (`bevy_camera/src/visibility/mod.rs:809` via
//! `ViewVisibility`; directional lights additionally filtered against the view's
//! layers at `bevy_pbr/src/render/light.rs:1804`), and cluster assignment
//! compares a clusterable object against the **view's** layers, not the light's
//! (`bevy_light/src/cluster/assign.rs:459`). Even `RectLight` carries a TODO for
//! the missing per-light filtering (`bevy_pbr/src/render/light.rs:1977`).
//!
//! What *is* filtered light-vs-mesh is **shadow casting**: `queue_shadows` skips
//! a mesh when the light's layers do not intersect the mesh's
//! (`bevy_pbr/src/render/light.rs:2633-2636`). So a default layer-0 light would
//! leave the split-off head copies out of every shadow map.
//!
//! Rather than leave that as a trap, [`VrmLightLayersPlugin`] widens lights that
//! carry no explicit layers to [`all_vrm_render_layers`], so a plain light lights
//! and shadows the exclusive meshes without the user doing anything. The
//! trade-off it cannot remove: a light whose layers the user sets explicitly is
//! left strictly alone, and that light will not shadow the exclusive meshes
//! unless its layers include 1 and 2.

use crate::macros::marker_component;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

/// The layer every entity belongs to unless it opts out.
///
/// This is bevy's default layer: an entity with no `RenderLayers` component is
/// treated as `RenderLayers::default() == RenderLayers::layer(0)`
/// (`bevy_camera/src/visibility/mod.rs:808`, `bevy_pbr/src/render/light.rs:2633`),
/// so layer 0 is also what a plain camera and a plain light live on.
pub const LAYER_BOTH: usize = 0;

/// The layer of a third-person-only mesh.
///
/// Kept disjoint from [`LAYER_BOTH`] on purpose: sharing layer 0 with a camera
/// would make the mesh visible to that camera. See the module docs.
pub const LAYER_THIRD_PERSON_ONLY: usize = 1;

/// The layer of a first-person-only mesh.
///
/// Kept disjoint from [`LAYER_BOTH`] on purpose: sharing layer 0 with a camera
/// would make the mesh visible to that camera. See the module docs.
pub const LAYER_FIRST_PERSON_ONLY: usize = 2;

/// Marks a head-mesh copy split off by `VRMC_vrm.firstPerson`'s `auto`
/// classification: the face/eyes/hair geometry that must be hidden from a
/// first-person camera.
///
/// The copy is created at load time and lives in the scene asset, so this
/// marker survives every instantiation of it. A plain marker — the render
/// layers on the entity already carry the visibility decision.
marker_component!(
    /// Marks a head-mesh copy split off by `firstPerson: auto`.
    VrmHeadOnly
);

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
    /// The blend weight; the constraint is skipped when this is `0.0`.
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

/// The render layers of a mesh visible in both views: layer 0 alone, which is
/// also bevy's default, so an unannotated mesh already has these.
#[must_use]
pub fn both_view_mesh_layers() -> RenderLayers {
    RenderLayers::from_layers(&[LAYER_BOTH])
}

/// The render layers of a third-person-only mesh: `{LAYER_THIRD_PERSON_ONLY}`.
///
/// Deliberately *not* `{LAYER_BOTH, LAYER_THIRD_PERSON_ONLY}` — see the module
/// docs. A light still shadows this mesh through [`all_vrm_render_layers`],
/// which [`VrmLightLayersPlugin`] applies automatically.
#[must_use]
pub fn third_person_only_mesh_layers() -> RenderLayers {
    RenderLayers::from_layers(&[LAYER_THIRD_PERSON_ONLY])
}

/// The render layers of a first-person-only mesh: `{LAYER_FIRST_PERSON_ONLY}`.
///
/// Deliberately *not* `{LAYER_BOTH, LAYER_FIRST_PERSON_ONLY}` — see the module
/// docs. A light still shadows this mesh through [`all_vrm_render_layers`],
/// which [`VrmLightLayersPlugin`] applies automatically.
#[must_use]
pub fn first_person_only_mesh_layers() -> RenderLayers {
    RenderLayers::from_layers(&[LAYER_FIRST_PERSON_ONLY])
}

/// The render layers of a first-person camera: `{LAYER_BOTH,
/// LAYER_FIRST_PERSON_ONLY}`.
///
/// Layer 0 is included so the camera also renders the rest of the scene, which
/// lives there.
#[must_use]
pub fn first_person_camera_layers() -> RenderLayers {
    RenderLayers::from_layers(&[LAYER_BOTH, LAYER_FIRST_PERSON_ONLY])
}

/// The render layers of a third-person camera: `{LAYER_BOTH,
/// LAYER_THIRD_PERSON_ONLY}`.
///
/// Layer 0 is included so the camera also renders the rest of the scene, which
/// lives there.
#[must_use]
pub fn third_person_camera_layers() -> RenderLayers {
    RenderLayers::from_layers(&[LAYER_BOTH, LAYER_THIRD_PERSON_ONLY])
}

/// The render layers that reach every VRM view: `{0, 1, 2}`.
///
/// A light needs this to shadow the exclusive meshes — `queue_shadows` drops a
/// mesh whose layers do not intersect the light's
/// (`bevy_pbr/src/render/light.rs:2633-2636`). Direct lighting does not need it
/// (see the module docs), but giving a light the extra layers costs nothing
/// visible, whereas omitting them silently costs shadows.
///
/// Prefer letting [`VrmLightLayersPlugin`] apply this for you: it is applied
/// automatically to any light that does not carry explicit layers.
#[must_use]
pub fn all_vrm_render_layers() -> RenderLayers {
    RenderLayers::from_layers(&[LAYER_BOTH, LAYER_THIRD_PERSON_ONLY, LAYER_FIRST_PERSON_ONLY])
}

/// Decides the render layers a newly added light should get.
///
/// Returns the layers to insert, or [`None`] to leave the light untouched.
///
/// The rule is deliberately narrow:
///
/// * no `RenderLayers` component at all — bevy reads that as
///   [`RenderLayers::default`], i.e. `{0}`, so this is the same case as the one
///   below and is widened;
/// * a `RenderLayers` equal to `{0}` — indistinguishable from absent, so this is
///   a default nobody asked for, and is widened;
/// * anything else — an explicit choice, left strictly alone.
///
/// Extracted from [`widen_vrm_light_layers`] so the rule is unit-testable
/// without spawning lights in a schedule.
#[must_use]
pub fn vrm_light_layers_for(existing: Option<&RenderLayers>) -> Option<RenderLayers> {
    // `Option::is_none_or` collapses the absent case and the explicit-`{0}`
    // case into the single "nobody asked for anything" branch.
    existing
        .is_none_or(|layers| layers == &RenderLayers::default())
        .then(all_vrm_render_layers)
}

/// Widens a newly spawned light to [`all_vrm_render_layers`] unless it already
/// carries explicit render layers, so that the exclusive first/third-person
/// meshes are not dropped from its shadow maps.
///
/// Runs once per light insertion, driven by `Added<..>` on the three light kinds
/// that can cast shadows in bevy 0.19.1. Because the trigger is insertion and
/// not `Changed<RenderLayers>`, a value the user sets later is never revisited.
///
/// One race is inherent to the deferred `Commands` insert: a user that adds
/// `RenderLayers` to the same light in the same frame the light component is
/// added may have the two inserts applied in insertion order, so the user's
/// value can win or lose depending on which system queued first. Inserting the
/// light first and the layers in a later frame avoids this.
pub fn widen_vrm_light_layers(
    mut commands: Commands,
    lights: Query<
        (Entity, Option<&RenderLayers>),
        Or<(Added<DirectionalLight>, Added<PointLight>, Added<SpotLight>)>,
    >,
) {
    for (entity, existing) in &lights {
        let Some(layers) = vrm_light_layers_for(existing) else {
            continue;
        };
        commands.entity(entity).insert(layers);
    }
}

/// Adds [`widen_vrm_light_layers`] to the app.
///
/// Covers the three light kinds that cast shadows in bevy 0.19.1. Two kinds are
/// deliberately excluded:
///
/// * [`AmbientLight`] is a per-camera override of `GlobalAmbientLight`
///   (`bevy_light/src/ambient_light.rs:11-12`, `#[require(Camera)]`), and
///   `prepare_lights` resolves it per **view** with no layer test at all
///   (`bevy_pbr/src/render/light.rs:1667-1672`), folding the result into a
///   per-view uniform (`:1756`). It lights every mesh the view renders whatever
///   the mesh's layers are, and widening it would only mutate the camera entity
///   it sits on.
/// * `RectLight` has no shadow map, and bevy does not use its render layers for
///   lighting at all yet — `bevy_pbr/src/render/light.rs:1977` is the standing
///   TODO. Widening it would change nothing today.
///
/// Adding this plugin is opt-in and app-wide: it changes layer membership for
/// every light in the app, not only those near a VRM. What that buys is that the
/// plugin's lights are also gathered for views on layers 1 and 2, which is what
/// a VRM camera needs.
pub struct VrmLightLayersPlugin;

impl Plugin for VrmLightLayersPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        // `PostUpdate` so lights spawned by `Startup`, `Update` and the usual
        // scene spawners are all seen in the same frame.
        app.add_systems(PostUpdate, widen_vrm_light_layers);
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
    /// (`bevy_world_serialization/src/world_asset_spawner.rs:505-519`).
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

    /// Bevy gates visibility on `RenderLayers::intersects`, a bitwise AND over
    /// the two layer sets — see `bevy_camera/src/visibility/mod.rs:809` and
    /// `bevy_pbr/src/render/mesh.rs:2294`. With no exclusion available, the
    /// exclusive mesh sets must be disjoint from the other camera's layers, so
    /// this pins the whole matrix.
    #[test]
    fn render_layer_helpers_pin_the_visibility_matrix() -> TestResult {
        assert_eq!(RenderLayers::default(), RenderLayers::layer(LAYER_BOTH));

        let both = both_view_mesh_layers();
        let third = third_person_only_mesh_layers();
        let first_person = first_person_only_mesh_layers();
        let third_camera = third_person_camera_layers();
        let first_camera = first_person_camera_layers();

        // The exclusive mesh sets really are disjoint from each other…
        assert!(!third.intersects(&first_person));
        // …and from the layer-0-only both-view mesh.
        assert!(!both.intersects(&third));
        assert!(!both.intersects(&first_person));

        // Each camera sees the both-view meshes…
        assert!(both.intersects(&third_camera));
        assert!(both.intersects(&first_camera));
        // …and only the meshes of its own view.
        assert!(third.intersects(&third_camera));
        assert!(!third.intersects(&first_camera));
        assert!(first_person.intersects(&first_camera));
        assert!(!first_person.intersects(&third_camera));

        // A plain camera sees the both-view meshes and nothing else, so these
        // helpers never take the rest of the scene away from an ordinary camera.
        assert!(both.intersects(&RenderLayers::default()));
        assert!(!third.intersects(&RenderLayers::default()));
        assert!(!first_person.intersects(&RenderLayers::default()));
        success!()
    }

    /// The exclusive meshes are what force the light obligation: bevy's
    /// `queue_shadows` drops a mesh whose layers do not intersect the light's
    /// (`bevy_pbr/src/render/light.rs:2633-2636`).
    #[test]
    fn a_default_light_shadows_nothing_exclusive_until_it_is_widened() -> TestResult {
        let default_light = RenderLayers::default();

        assert!(default_light.intersects(&both_view_mesh_layers()));
        assert!(!default_light.intersects(&third_person_only_mesh_layers()));
        assert!(!default_light.intersects(&first_person_only_mesh_layers()));

        let widened = all_vrm_render_layers();
        assert!(widened.intersects(&both_view_mesh_layers()));
        assert!(widened.intersects(&third_person_only_mesh_layers()));
        assert!(widened.intersects(&first_person_only_mesh_layers()));

        // Widening must not cost the light the ordinary layer-0 scene either.
        assert!(widened.intersects(&RenderLayers::default()));
        assert!(widened.intersects(&third_person_camera_layers()));
        assert!(widened.intersects(&first_person_camera_layers()));
        success!()
    }

    /// The decision function is the whole rule, so it is tested on its own.
    /// `widen_light_layers_in_a_running_app` covers the wiring.
    #[test]
    fn absent_and_default_light_layers_are_widened() -> TestResult {
        assert_eq!(vrm_light_layers_for(None), Some(all_vrm_render_layers()));
        assert_eq!(
            vrm_light_layers_for(Some(&RenderLayers::default())),
            Some(all_vrm_render_layers())
        );
        success!()
    }

    #[test]
    fn explicit_light_layers_are_left_alone() -> TestResult {
        for explicit in [
            RenderLayers::layer(LAYER_FIRST_PERSON_ONLY),
            RenderLayers::layer(LAYER_THIRD_PERSON_ONLY),
            first_person_camera_layers(),
            third_person_camera_layers(),
            RenderLayers::none(),
        ] {
            assert_eq!(vrm_light_layers_for(Some(&explicit)), None);
        }
        success!()
    }

    /// Drives the real system through a real schedule: no render app needed,
    /// because layer widening is pure ECS work.
    #[test]
    fn widen_light_layers_in_a_running_app() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmLightLayersPlugin);

        // No `RenderLayers` at all, and an explicit non-default one.
        let bare = app.world_mut().spawn(DirectionalLight::default()).id();
        let explicit = RenderLayers::layer(LAYER_FIRST_PERSON_ONLY);
        let chosen = app
            .world_mut()
            .spawn((SpotLight::default(), explicit.clone()))
            .id();
        // Not a light: must be ignored entirely.
        let mesh = app.world_mut().spawn_empty().id();

        app.update();

        assert_eq!(
            app.world().get::<RenderLayers>(bare),
            Some(&all_vrm_render_layers())
        );
        assert_eq!(app.world().get::<RenderLayers>(chosen), Some(&explicit));
        assert_eq!(app.world().get::<RenderLayers>(mesh), None);

        // The system must not fight the user: a value set after the light was
        // widened survives, because the trigger is `Added<..>`, not
        // `Changed<RenderLayers>`.
        let narrowed = RenderLayers::layer(LAYER_THIRD_PERSON_ONLY);
        app.world_mut().entity_mut(bare).insert(narrowed.clone());
        app.update();

        assert_eq!(app.world().get::<RenderLayers>(bare), Some(&narrowed));
        success!()
    }
}
