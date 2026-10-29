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
use bevy::asset::{Handle, LoadContext, RenderAssetUsages};
use bevy::ecs::entity::Entity;
use bevy::ecs::world::{EntityWorldMut, World};
use bevy::gltf::convert_coordinates::GltfConvertCoordinates;
use bevy::gltf::extensions::{ErasedGltfExtensionHandler, GltfExtensionHandler};
use bevy::gltf::{GltfLoaderSettings, GltfMaterial};
use bevy::mesh::{Mesh, MeshCompressionArgs, MeshVertexAttribute};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::ConditionalSendFuture;
use gltf::Node;
use std::sync::Arc;

use crate::error::vrm_warn;
use crate::prelude::MToonMaterial;
use crate::prelude::*;
use crate::vrm::gltf::extensions::{vrmc_spring_bone, vrmc_vrm};

/// Registered once by `VrmGltfPlugin`, which [`VrmPlugin`](crate::prelude::VrmPlugin) adds.
#[derive(Clone)]
pub struct VrmExtensionHandler {
    pub(crate) state: VrmLoadState,
    /// `GltfPlugin::convert_coordinates`: the conversion a load uses when its
    /// settings name none, i.e. what the stock `GltfLoader` applies to a `.glb`.
    pub(crate) default_convert_coordinates: GltfConvertCoordinates,
    /// `GltfPlugin::mesh_compression`: the compression a load uses when its
    /// settings name none.
    pub(crate) default_mesh_compression: MeshCompressionArgs,
}

impl Default for VrmExtensionHandler {
    fn default() -> Self {
        Self {
            state: VrmLoadState::default(),
            default_convert_coordinates: GltfConvertCoordinates::default(),
            default_mesh_compression: MeshCompressionArgs::none(),
        }
    }
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
    /// Whether this load converts mesh coordinates (`rotate_meshes`).
    ///
    /// `VrmLoader` never does, but the stock `GltfLoader` runs this handler too
    /// and may. `bevy_gltf` then converts its own meshes and every skin's
    /// inverse bind matrices, while the `firstPerson: auto` split rebuilds its
    /// halves from the raw accessors, which would leave them rotated 180° against
    /// their skeleton; the split is skipped instead.
    pub rotate_meshes: bool,
    /// `GltfLoaderSettings::load_meshes`, which the `firstPerson: auto` split
    /// creates both of its halves with, like every mesh `bevy_gltf` builds.
    pub load_meshes: RenderAssetUsages,
    /// The compression `bevy_gltf` applies to this load's meshes, which the
    /// `firstPerson: auto` split applies to the head half it adds itself.
    /// `None` (before `on_root`) compresses nothing.
    pub mesh_compression: Option<MeshCompressionArgs>,
    /// glTF material label -> the `MToonMaterial` built for it.
    pub mtoon: HashMap<String, Handle<MToonMaterial>>,
    /// The number of glTF textures, which bounds the `MToon` texture indices.
    pub texture_count: usize,
    /// glTF texture index -> the image handle `bevy_gltf` created for it, as
    /// handed to `on_texture`.
    ///
    /// Only an *embedded* image becomes a `Texture{index}` labeled asset; one
    /// stored as a separate file is loaded by path under its own asset path, so
    /// a label handle for it would never load. These are the handles that do.
    pub textures: HashMap<usize, Handle<Image>>,
    /// firstPerson classification/split per (mesh index, primitive index).
    pub first_person: first_person::FirstPersonPlan,
    /// (mesh index, primitive index) -> the entities carrying its `Mesh3d`, one
    /// per node that instances the mesh, so the per-primitive decisions can be
    /// applied to exactly those entities. Cleared by [`Self::end_scene`].
    pub primitive_entities: HashMap<(usize, usize), Vec<Entity>>,
}

impl VrmLoadState {
    /// Whether `on_root` found a `VRMC_vrm` model in this file.
    ///
    /// `VRMC_springBone` alone does not make a file a VRM: the extension is also
    /// used by props and can ride along in a `.vrma`, and treating such a file
    /// as an avatar would rename its scene, add a player and, for a `.vrma`,
    /// get its instance cleaned up as a stale VRM spawn. A file whose
    /// `VRMC_vrm` fails to parse is a plain glTF, as `root.rs` documents.
    #[inline]
    #[must_use]
    pub fn is_vrm(&self) -> bool {
        self.vrm.is_some()
    }

    /// Forgets the entities of the scene that just completed: `bevy_gltf`
    /// builds every scene of a file in a fresh `World`, where they mean nothing.
    pub(crate) fn end_scene(&mut self) {
        self.node_entities.clear();
        self.primitive_entities.clear();
    }
}

/// Whether a load with `settings` converts mesh coordinates, falling back to
/// the plugin's `default` the way `bevy_gltf` does.
fn rotates_meshes(
    settings: &GltfLoaderSettings,
    default: GltfConvertCoordinates,
) -> bool {
    settings
        .convert_coordinates
        .unwrap_or(default)
        .rotate_meshes
}

/// The normalized animation target of a humanoid bone.
///
/// Identical to `AnimationTargetId::from_name(&Name::new(bone))`, spelled out
/// because every consumer of a retargeted `.vrma` clip needs to agree on it.
#[must_use]
pub(crate) fn bone_target_id(bone: &str) -> AnimationTargetId {
    AnimationTargetId::from_iter([bone])
}

impl GltfExtensionHandler for VrmExtensionHandler {
    fn dyn_clone(&self) -> Box<dyn ErasedGltfExtensionHandler> {
        Box::new(self.clone())
    }

    fn on_root(
        &mut self,
        _load_context: &mut LoadContext<'_>,
        gltf: &gltf::Gltf,
        settings: &GltfLoaderSettings,
    ) {
        root::parse_root_extensions(&mut self.state, gltf);
        self.state.rotate_meshes = rotates_meshes(settings, self.default_convert_coordinates);
        self.state.load_meshes = settings.load_meshes;
        self.state.mesh_compression = Some(
            settings
                .mesh_compression
                .clone()
                .unwrap_or_else(|| self.default_mesh_compression.clone()),
        );
        if self.state.is_vrm() && self.state.rotate_meshes {
            vrm_warn!(
                "VRM: this file is loaded with `rotate_meshes` coordinate conversion, which the \
                 `firstPerson: auto` split cannot honour; mixed head/body meshes are left unsplit"
            );
        }
    }

    fn on_texture(
        &mut self,
        gltf_texture: &gltf::Texture,
        texture: Handle<Image>,
    ) {
        if !self.state.is_vrm() {
            return;
        }
        self.state.textures.insert(gltf_texture.index(), texture);
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
