//! [`VrmExtensionHandler`]: the trait implementation that hooks VRM handling
//! into the stock glTF loader.
//!
//! # Why every hook starts with `is_vrm()`
//!
//! [`GltfExtensionHandlers`] is a single app-wide registry shared by *every*
//! glTF loader, so a `.glb` loaded by the stock `GltfLoader` runs this handler
//! too. `on_root` is the only hook that decides whether a file is a VRM, and it
//! records the answer in [`VrmLoadState`]; every other hook returns immediately
//! when the answer is no.
//!
//! # Why `state` is per-load and starts empty
//!
//! `bevy_gltf` clones the handler list once per file load
//! (`crates/bevy_gltf/src/loader/mod.rs`,
//! `loader.extensions.read().await.clone()`) and then calls `on_root` on the
//! clone. `dyn_clone` therefore hands out
//! `Box::new(self.clone())` of the *pristine* registered instance, whose
//! `state` is `Default::default()` — so every load starts with an empty state
//! and two concurrent loads cannot see each other's mappings.
//!
//! # What is deliberately not here
//!
//! `on_animation` and `on_animations_collected` are **not** overridden: baking
//! `.vrma` clips is a separate commit. The state is laid out so that commit only
//! adds fields (the raw GLB BIN chunk and the animation graph handle) and two
//! hook bodies, without touching the load-time steps.

pub mod first_person;
pub mod materials;
pub mod nodes;
pub mod root;
pub mod scene;

use bevy::animation::AnimationTargetId;
use bevy::asset::{Handle, LoadContext};
use bevy::ecs::entity::Entity;
use bevy::ecs::world::{EntityWorldMut, World};
use bevy::gltf::extensions::{ErasedGltfExtensionHandler, GltfExtensionHandler};
use bevy::gltf::{GltfLoaderSettings, GltfMaterial};
use bevy::mesh::{Mesh, MeshVertexAttribute};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::ConditionalSendFuture;
use gltf::Node;
use std::sync::Arc;

use crate::prelude::MToonMaterial;
use crate::prelude::*;
use crate::vrm::gltf::extensions::{vrmc_spring_bone, vrmc_vrm};

use self::root::NodeRest;

/// Registered once in [`VrmGltfPlugin::build`](crate::vrm::gltf::VrmGltfPlugin).
#[derive(Clone, Default)]
pub struct VrmExtensionHandler {
    pub(crate) state: VrmLoadState,
}

/// The state of one file load.
///
/// Everything in here is keyed by **glTF index**, never by
/// [`Name`]: glTF node names are optional and routinely collide inside one
/// file. It is resolved to [`Entity`] in
/// [`scene`](self::scene), which is the only place that sees the scene world.
#[derive(Clone, Default)]
pub(crate) struct VrmLoadState {
    /// Parsed `VRMC_vrm`; `None` means "not a VRM file".
    ///
    /// Behind an `Arc` because the handler has to be `Clone` (`dyn_clone` clones
    /// the registry per file load) while the shared schemas deliberately are not
    /// `Clone`: they are only ever read here.
    pub vrm: Option<Arc<vrmc_vrm::VrmcVrm>>,
    /// Parsed `VRMC_springBone`. See [`Self::vrm`] for the `Arc`.
    pub spring_bone: Option<Arc<vrmc_spring_bone::VRMCSpringBone>>,
    /// glTF node index -> entity in the world of the scene being built (filled
    /// in `on_gltf_node`, cleared by [`Self::end_scene`]).
    pub node_entities: HashMap<usize, Entity>,
    /// VRM humanoid bone name (`hips`, `head`, …) -> glTF node index.
    pub bone_nodes: HashMap<String, usize>,
    /// glTF mesh index -> the skin index of the node that instances it.
    ///
    /// `None` means "no skin, or ambiguous"; see
    /// [`root::collect_mesh_skins`](self::root) for why the skin cannot be read
    /// from the entity's own node in `on_gltf_primitive`.
    pub mesh_skins: HashMap<usize, Option<usize>>,
    /// Node indices of the head subtree, for the `firstPerson: auto` split.
    pub head_subtree: HashSet<usize>,
    /// Rest pose of **every** glTF node, keyed by node index.
    #[allow(
        dead_code,
        reason = "captured for the hips-scaling and .vrma commits, which measure against it"
    )]
    pub node_rest: HashMap<usize, NodeRest>,
    /// The uniform scale declared by the scene's root node. See
    /// [`NodeRest::world_translation_without_root_scale`].
    #[allow(
        dead_code,
        reason = "read together with `node_rest`, which the hips-scaling commit consumes"
    )]
    pub root_scale: f32,
    /// glTF material label -> the `MToonMaterial` built for it.
    pub mtoon: HashMap<String, Handle<MToonMaterial>>,
    /// The number of glTF textures, which bounds the `MToon` texture indices.
    pub texture_count: usize,
    /// firstPerson classification/split per (mesh index, primitive index).
    pub first_person: first_person::FirstPersonPlan,
    /// (mesh index, primitive index) -> the entities carrying its `Mesh3d`, one
    /// per node that instances the mesh, so the per-primitive decisions can be
    /// applied to exactly those entities. Cleared by [`Self::end_scene`].
    pub primitive_entities: HashMap<(usize, usize), Vec<Entity>>,
}

impl VrmLoadState {
    /// Whether `on_root` found any VRM extension in this file.
    #[inline]
    #[must_use]
    pub fn is_vrm(&self) -> bool {
        self.vrm.is_some() || self.spring_bone.is_some()
    }

    /// Forgets the entities of the scene that just completed: `bevy_gltf`
    /// builds every scene of a file in a fresh `World`, where they mean nothing.
    pub(crate) fn end_scene(&mut self) {
        self.node_entities.clear();
        self.primitive_entities.clear();
    }

    /// `(humanoid bone name, AnimationTargetId)` pairs for graph mask groups.
    ///
    /// The id is derived from the **bone name**, not from the glTF node path.
    /// That is what makes one baked `.vrma` clip retargetable to any avatar:
    /// the clip's curves are keyed by `bones.hips` etc. and the avatar's bones
    /// are keyed by the same string. The path-based id that `bevy_gltf` uses for
    /// a file's own animations is a different id for the same entity and is
    /// left alone by [`scene::setup_animation`]; see
    /// [`NodeRest::path`] for how that one is built.
    ///
    /// Sorted by bone name so the graph a file produces does not depend on
    /// `HashMap` iteration order.
    #[allow(
        dead_code,
        reason = "the graph builder of the .vrma commit is the consumer"
    )]
    pub fn bone_targets(&self) -> impl Iterator<Item = (&str, AnimationTargetId)> + '_ {
        let mut bones: Vec<&str> = self.bone_nodes.keys().map(String::as_str).collect();
        bones.sort_unstable();
        bones.into_iter().map(|bone| (bone, bone_target_id(bone)))
    }
}

/// The normalized animation target of a humanoid bone.
///
/// Identical to `AnimationTargetId::from_name(&Name::new(bone))`, spelled out
/// because every consumer of a retargeted `.vrma` clip needs to agree on it.
#[must_use]
pub(crate) fn bone_target_id(bone: &str) -> AnimationTargetId {
    // `AnimationTargetId::from_iter` hashes the strings directly, which is what
    // `from_name(&Name::new(..))` does, but without needing an owned
    // `CowArc<'static, str>`.
    AnimationTargetId::from_iter([bone])
}

impl GltfExtensionHandler for VrmExtensionHandler {
    fn dyn_clone(&self) -> Box<dyn ErasedGltfExtensionHandler> {
        // See the module docs: `bevy_gltf` clones the registry per file load, so
        // this hands out a fresh `Default` state every time.
        Box::new(self.clone())
    }

    fn on_root(
        &mut self,
        _load_context: &mut LoadContext<'_>,
        gltf: &gltf::Gltf,
        _settings: &GltfLoaderSettings,
    ) {
        root::parse_root_extensions(&mut self.state, gltf);
    }

    fn on_material(
        &mut self,
        load_context: &mut LoadContext<'_>,
        gltf_material: &gltf::Material,
        _material: Handle<GltfMaterial>,
        material_asset: &GltfMaterial,
        material_label: &str,
    ) {
        if !self.state.is_vrm() {
            return;
        }
        materials::process_material(
            &mut self.state,
            load_context,
            gltf_material,
            material_asset,
            material_label,
        );
    }

    fn on_gltf_primitive(
        &mut self,
        load_context: &mut LoadContext<'_>,
        gltf_document: &gltf::Gltf,
        gltf_mesh: &gltf::Mesh,
        gltf_primitive: &gltf::Primitive,
        buffer_data: &[Vec<u8>],
        _custom_vertex_attributes: &HashMap<Box<str>, MeshVertexAttribute>,
        gltf_mesh_on_skinned_nodes: bool,
        _gltf_mesh_on_non_skinned_nodes: bool,
        user_mesh: &mut Option<Mesh>,
    ) -> impl ConditionalSendFuture<Output = ()> {
        // Every step here is synchronous and has to happen *before* the empty
        // future is returned, so no `&mut self.state` borrow can escape into it.
        if self.state.is_vrm() && gltf_mesh_on_skinned_nodes {
            first_person::process_primitive(
                &mut self.state,
                load_context,
                gltf_document,
                gltf_mesh,
                gltf_primitive,
                buffer_data,
                user_mesh,
            );
        }
        async {}
    }

    fn on_spawn_mesh_and_material(
        &mut self,
        _load_context: &mut LoadContext<'_>,
        primitive: &gltf::Primitive,
        mesh: &gltf::Mesh,
        _material: &gltf::Material,
        entity: &mut EntityWorldMut,
        material_label: &str,
    ) {
        if !self.state.is_vrm() {
            return;
        }
        // `on_spawn_mesh_and_material` runs once per node that instances this
        // (mesh, primitive), and each of those entities gets the per-primitive
        // classification and the head split.
        self.state
            .primitive_entities
            .entry((mesh.index(), primitive.index()))
            .or_default()
            .push(entity.id());
        materials::swap_material(&self.state, entity, material_label);
    }

    fn on_gltf_node(
        &mut self,
        _load_context: &mut LoadContext<'_>,
        gltf_node: &Node,
        entity: &mut EntityWorldMut,
    ) {
        if !self.state.is_vrm() {
            return;
        }
        nodes::process_node(&mut self.state, gltf_node, entity);
    }

    fn on_scene_completed(
        &mut self,
        load_context: &mut LoadContext<'_>,
        scene: &gltf::Scene,
        world_root_id: Entity,
        scene_world: &mut World,
    ) {
        if !self.state.is_vrm() {
            return;
        }
        scene::finalize(
            &mut self.state,
            load_context,
            scene,
            world_root_id,
            scene_world,
        );
        self.state.end_scene();
    }
}
