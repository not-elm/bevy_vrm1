//! `on_root`: the root-level VRM extensions, parsed into the per-load state.
//!
//! The extension JSON is read straight off the [`gltf::Gltf`] the hook is
//! handed — `gltf::Document::extensions()` and friends — because a handler never
//! sees bevy's [`Gltf`] asset and therefore cannot use
//! [`VrmExtensions`](crate::vrm::gltf::extensions::VrmExtensions). The schemas
//! themselves are the crate's shared ones
//! ([`vrmc_vrm::VrmcVrm`], [`vrmc_spring_bone::VRMCSpringBone`]), the same ones a
//! `.vrma` file's own extensions deserialize into, so there is no second set of
//! VRM structs to keep in sync.
//!
//! # `VRMC_vrm_animation` is deliberately *not* a model root
//!
//! VRM 0.0 files put their model extensions under the name `VRMC_vrm_animation`,
//! which [`obtain_vrmc_vrm`](crate::vrm::gltf::extensions::obtain_vrmc_vrm)
//! accepts as a fallback. This loader does not: `VRMC_vrm_animation-1.0` is the
//! root of a `.vrma` file, whose `expressions` map holds *node references*
//! (`{"happy": {"node": 5}}`) where `VRMC_vrm` holds expression *definitions*.
//! Both deserialize into `VrmcVrm` — the reference parses as an empty
//! expression — so accepting the name would silently turn every `.vrma` file
//! loaded through [`VrmaLoaderPlugin`](crate::vrma::loader::VrmaLoaderPlugin)
//! into a VRM scene with no expressions. Only `VRMC_vrm` marks a file as a VRM.

use bevy::animation::AnimationTargetId;
use bevy::math::{Mat4, Quat, Vec3};
use bevy::platform::collections::hash_map::Entry;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use std::sync::Arc;

use super::VrmLoadState;
use crate::error::vrm_warn;
use crate::vrm::gltf::extensions::vrmc_spring_bone::VRMCSpringBone;
use crate::vrm::gltf::extensions::vrmc_vrm::VrmcVrm;

/// The model extension, `VRMC_vrm-1.0`.
pub const EXT_VRMC_VRM: &str = "VRMC_vrm";
/// The spring-bone extension, `VRMC_springBone-1.0`.
pub const EXT_VRMC_SPRING_BONE: &str = "VRMC_springBone";
/// The node extension, `VRMC_node_constraint-1.0`.
pub const EXT_NODE_CONSTRAINT: &str = "VRMC_node_constraint";

/// The VRM humanoid bone whose subtree defines `firstPerson: auto`.
const HEAD_BONE: &str = "head";

/// The rest pose of one glTF node, captured from the document.
///
/// Every field is in the glTF file's own space — *without* bevy's scene
/// conversion, which `bevy_gltf` applies to the scene root entity rather than
/// to any node. For transforms that already exist on an entity prefer the
/// components written by [`insert_rest_transforms`](super::scene); this map
/// exists for the measurements that must happen while the document is still in
/// hand.
#[allow(
    dead_code,
    reason = "the hips-scaling and .vrma commits are the readers; the fields are pinned by the tests below"
)]
#[derive(Clone, Debug, PartialEq)]
pub struct NodeRest {
    /// The node's **local** rest transform, identical to the `Transform` bevy
    /// puts on the node entity (see [`node_transform`], a mirror of bevy's).
    ///
    /// Node constraints measure the source's rotation *relative to its rest*, so
    /// this is the reference of that delta.
    pub rest: Transform,
    /// The node's **world** rest transform, composed down from its topmost
    /// ancestor. Identical to `RestGlobalTransform` in a file whose scene root
    /// carries no conversion transform.
    pub world: GlobalTransform,
    /// The **parent's** world rotation at rest.
    ///
    /// `VRMC_node_constraint`'s `roll` and `aim` variants both state their axis
    /// in the *source node's* rest space and then express it through the
    /// destination's parent frame, so a constraint needs the parent rotation of
    /// the source before any runtime transform has been sampled.
    pub parent_world_rotation: Quat,
    /// The node's path of [`Name`]s, built exactly the way `bevy_gltf` builds the
    /// path of its own animation target ids.
    ///
    /// `bevy_gltf` collects it in
    /// `loader/gltf_ext/scene.rs::collect_path`, which pushes the animation
    /// root's own name first and then every descendant's, and hashes it with
    /// `AnimationTargetId::from_names`. A clip that was authored against the
    /// `.vrm`'s own glTF animations addresses its bones by that id, so the
    /// matching id has to be reconstructible — see [`Self::path_target_id`].
    pub path: Vec<Name>,
}

impl NodeRest {
    /// The path-based `AnimationTargetId` `bevy_gltf` would put on this node.
    ///
    /// **This is not** the id used for retargeting: see
    /// [`bone_target_id`](super::bone_target_id) for the bone-name id that
    /// `.vrma` curves use.
    #[allow(
        dead_code,
        reason = "the .vrma commit reads a clip authored against the .vrm's own animations"
    )]
    #[must_use]
    pub fn path_target_id(&self) -> AnimationTargetId {
        AnimationTargetId::from_names(self.path.iter())
    }

    /// The rest world translation with the file's uniform root scale divided out.
    ///
    /// VRM 1.0 requires the scene root's scale to be uniform and uses the
    /// avatar's own units, so a hips-height comparison has to happen in a
    /// metric space where one unit is one metre. Every rest pose stored on an
    /// entity stays in file space (that is what a later
    /// [`RestGlobalTransform`](crate::vrm::RestGlobalTransform) is), which is why
    /// the removal is an explicit step here rather than baked into
    /// [`Self::world`].
    #[allow(
        dead_code,
        reason = "the hips-scaling commit compares against the target height in this space"
    )]
    #[must_use]
    pub fn world_translation_without_root_scale(
        &self,
        root_scale: f32,
    ) -> Vec3 {
        if root_scale == 0.0 || !root_scale.is_finite() {
            return self.world.translation();
        }
        self.world.translation() / root_scale
    }
}

/// Parses the root extensions of one file into `state`.
///
/// Every failure is a warning: a `.vrm` that loses its spring bones still has an
/// avatar, and one that loses `VRMC_vrm` entirely is treated as a plain glTF.
pub(crate) fn parse_root_extensions(
    state: &mut VrmLoadState,
    gltf: &gltf::Gltf,
) {
    state.vrm = parse_ext::<VrmcVrm>(gltf, EXT_VRMC_VRM).map(Arc::new);
    state.spring_bone = parse_ext::<VRMCSpringBone>(gltf, EXT_VRMC_SPRING_BONE).map(Arc::new);

    if !state.is_vrm() {
        return;
    }

    if let Some(vrm) = &state.vrm {
        for (bone, human_bone) in &vrm.humanoid.human_bones {
            state.bone_nodes.insert(bone.clone(), human_bone.node);
        }
        // The head bone is required by the schema, but a file that does not
        // declare it must still load; the `auto` split then has no head subtree
        // and every mesh stays visible in both views.
        if let Some(head) = vrm.humanoid.human_bones.get(HEAD_BONE) {
            state.head_subtree = collect_subtree(gltf, head.node);
        }
    }

    state.mesh_skins = collect_mesh_skins(gltf);
    state.texture_count = gltf.textures().len();

    let rest = collect_rest_poses(gltf);
    state.root_scale = rest.root_scale;
    state.node_rest = rest.nodes;
}

fn parse_ext<T: serde::de::DeserializeOwned>(
    gltf: &gltf::Gltf,
    name: &str,
) -> Option<T> {
    let value = gltf.document.extensions()?.get(name)?;
    match serde_json::from_value(value.clone()) {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            vrm_warn!(format!(
                "VRM: failed to parse `{name}`, skipping it: {error}"
            ));
            None
        }
    }
}

/// Every node index in the subtree of `root`, inclusive.
fn collect_subtree(
    gltf: &gltf::Gltf,
    root: usize,
) -> HashSet<usize> {
    fn walk(
        node: &gltf::Node<'_>,
        out: &mut HashSet<usize>,
    ) {
        // A cyclic node hierarchy makes `bevy_gltf` fail the whole load
        // (`check_for_cycles`), but this runs *before* that check, so the
        // recursion is bounded rather than trusted.
        if !out.insert(node.index()) {
            return;
        }
        for child in node.children() {
            walk(&child, out);
        }
    }

    let mut out = HashSet::default();
    let nodes: Vec<_> = gltf.document.nodes().collect();
    if let Some(node) = nodes.get(root) {
        walk(node, &mut out);
    } else {
        vrm_warn!(format!(
            "VRM: the `{HEAD_BONE}` bone points at node {root}, which does not exist; \
             `firstPerson: auto` meshes stay visible in both views"
        ));
    }
    out
}

/// glTF mesh index -> the skin index of the node that instances it.
///
/// `on_gltf_primitive` is called once per `(mesh, primitive)` and is **not**
/// given the glTF node, so the skin cannot be read from "the entity's own node"
/// at that point — the node only exists later, in `on_gltf_node`. Indexing the
/// skins by mesh index here is the closest faithful answer, and it is still
/// strictly better than picking the first node that references the mesh: that
/// node may carry no skin at all, which silently classified every `auto` mesh
/// of such a file as `both`.
///
/// A mesh instanced by several nodes with *different* skins has no single
/// answer; it is recorded as unskinned, which is the same `both` fallback
/// `handler::first_person::classify` reaches when a mesh has no usable joint
/// indices.
fn collect_mesh_skins(gltf: &gltf::Gltf) -> HashMap<usize, Option<usize>> {
    let mut skins: HashMap<usize, Option<usize>> = HashMap::default();
    for node in gltf.document.nodes() {
        let Some(mesh) = node.mesh() else {
            continue;
        };
        let skin = node.skin().map(|skin| skin.index());
        match skins.entry(mesh.index()) {
            Entry::Vacant(entry) => {
                entry.insert(skin);
            }
            Entry::Occupied(mut entry) => {
                if entry.get() != &skin {
                    vrm_warn!(format!(
                        "VRM firstPerson: mesh {} is instanced by nodes with different skins; \
                         treating it as unskinned",
                        mesh.index()
                    ));
                    entry.insert(None);
                }
            }
        }
    }
    skins
}

/// The rest pose of every node, plus the scene's uniform root scale.
struct RestPoses {
    nodes: HashMap<usize, NodeRest>,
    root_scale: f32,
}

fn collect_rest_poses(gltf: &gltf::Gltf) -> RestPoses {
    let nodes: Vec<_> = gltf.document.nodes().collect();

    // `gltf::Node` has no `parent()`, so the parent of every node is collected
    // in one pass over the child lists.
    let parents: HashMap<usize, usize> = nodes
        .iter()
        .flat_map(|node| node.children().map(|child| (child.index(), node.index())))
        .collect();

    // Compose world transforms top-down. Sorting by depth makes the parent of
    // every node available before the node itself, and the walk bound keeps a
    // cyclic document from looping forever (`bevy_gltf` rejects cycles later,
    // but this runs before that check).
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|index| depth_of(*index, &parents, nodes.len()));

    let mut worlds: HashMap<usize, GlobalTransform> = HashMap::default();
    let mut rests: HashMap<usize, NodeRest> = HashMap::default();
    for index in order {
        let Some(node) = nodes.get(index) else {
            continue;
        };
        let rest = node_transform(node);
        let parent = parents
            .get(&index)
            .and_then(|parent| worlds.get(parent))
            .copied();
        let world = match parent {
            Some(parent) => parent.mul_transform(rest),
            None => GlobalTransform::from(rest),
        };
        let parent_world_rotation = parent.map_or(Quat::IDENTITY, |parent| {
            parent.to_scale_rotation_translation().1
        });
        rests.insert(
            index,
            NodeRest {
                rest,
                world,
                parent_world_rotation,
                path: node_path(index, &parents, &nodes),
            },
        );
        worlds.insert(index, world);
    }

    RestPoses {
        nodes: rests,
        root_scale: uniform_root_scale(gltf),
    }
}

/// How many ancestors `index` has, bounded by `limit`.
fn depth_of(
    index: usize,
    parents: &HashMap<usize, usize>,
    limit: usize,
) -> usize {
    let mut depth = 0;
    let mut cursor = index;
    while let Some(parent) = parents.get(&cursor).copied() {
        if depth >= limit {
            return depth;
        }
        cursor = parent;
        depth += 1;
    }
    depth
}

/// The node's own name followed by its ancestors', i.e. the path
/// `bevy_gltf`'s `collect_path` would build for it.
fn node_path(
    index: usize,
    parents: &HashMap<usize, usize>,
    nodes: &[gltf::Node<'_>],
) -> Vec<Name> {
    let mut path = vec![node_name(index, nodes)];
    let mut cursor = index;
    let mut walked = 0;
    while let Some(parent) = parents.get(&cursor).copied() {
        if walked > nodes.len() {
            break;
        }
        path.push(node_name(parent, nodes));
        cursor = parent;
        walked += 1;
    }
    path.reverse();
    path
}

fn node_name(
    index: usize,
    nodes: &[gltf::Node<'_>],
) -> Name {
    nodes
        .get(index)
        .map_or_else(|| Name::new(format!("GltfNode{index}")), node_name_of)
}

/// Mirrors `bevy_gltf`'s `loader/gltf_ext/scene.rs::node_name`, so a path built
/// here hashes to the same `AnimationTargetId` as the one bevy builds.
fn node_name_of(node: &gltf::Node<'_>) -> Name {
    Name::new(
        node.name()
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("GltfNode{}", node.index())),
    )
}

/// Mirrors `bevy_gltf`'s `loader/gltf_ext/scene.rs::node_transform`, so the rest
/// pose captured here is bit-for-bit the `Transform` bevy puts on the entity.
fn node_transform(node: &gltf::Node<'_>) -> Transform {
    match node.transform() {
        gltf::scene::Transform::Matrix { matrix } => {
            Transform::from_matrix(Mat4::from_cols_array_2d(&matrix))
        }
        gltf::scene::Transform::Decomposed {
            translation,
            rotation,
            scale,
        } => Transform {
            translation: Vec3::from(translation),
            rotation: Quat::from_array(rotation),
            scale: Vec3::from(scale),
        },
    }
}

/// The uniform scale of the first node of the first scene.
///
/// `1.0` when the file has no scene, when the root node has no uniform scale, or
/// when that scale is zero — in each case the rest poses are already in the
/// space the caller asked for.
fn uniform_root_scale(gltf: &gltf::Gltf) -> f32 {
    let Some(node) = gltf
        .document
        .scenes()
        .next()
        .and_then(|scene| scene.nodes().next())
    else {
        return 1.0;
    };
    let (_, _, scale) = node.transform().decomposed();
    let scale = Vec3::from(scale);
    if (scale - Vec3::splat(scale.x)).length() > 1e-5 {
        vrm_warn!(format!(
            "VRM: the scene root node declares a non-uniform scale {scale:?}; the specification \
             requires a uniform one, so it is left in place"
        ));
        return 1.0;
    }
    if scale.x == 0.0 || !scale.x.is_finite() {
        vrm_warn!(format!(
            "VRM: the scene root node declares a zero scale {scale:?}; ignoring it"
        ));
        return 1.0;
    }
    scale.x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::gltf::handler::VrmLoadState;

    /// Parses one of the crate's own avatars straight off disk.
    ///
    /// The glTF loader is asynchronous, so a `cargo test` binary cannot drive a
    /// full `.vrm` load (see the note in `crate::vrm::gltf`'s test module);
    /// reading the same bytes the loader would and handing them to the same
    /// `on_root` entry point covers everything the hook decides.
    fn avatar(name: &str) -> gltf::Gltf {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/vrm")
            .join(name);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("`{}` should be readable: {error}", path.display()));
        gltf::Gltf::from_slice_without_validation(&bytes)
            .unwrap_or_else(|error| panic!("`{}` should parse: {error}", path.display()))
    }

    /// The acceptance test at the hook level: the avatar the examples load goes
    /// through `on_root` and produces the state the rest of the pipeline needs.
    #[test]
    fn elmer_produces_the_state_the_pipeline_needs() -> TestResult {
        let gltf = avatar("Elmer.vrm");
        let mut state = VrmLoadState::default();

        parse_root_extensions(&mut state, &gltf);

        assert!(state.is_vrm());
        assert!(state.spring_bone.is_some(), "Elmer has hair physics");

        // The humanoid map, addressed by VRM bone name.
        assert_eq!(state.bone_nodes.len(), 54);
        assert_eq!(state.bone_nodes.get("hips"), Some(&1));
        let head = *state.bone_nodes.get("head").expect("`head` is required");
        assert!(
            state.head_subtree.contains(&head),
            "the head is its own root"
        );
        assert!(
            state.head_subtree.len() > 1,
            "the head subtree carries the face, eyes and hair bones"
        );

        // Every skin is resolvable per mesh, which is what the `firstPerson`
        // classification needs.
        let skinned = state
            .mesh_skins
            .values()
            .filter(|skin| skin.is_some())
            .count();
        assert_eq!(skinned, 3, "all three of Elmer's meshes are skinned");

        // The rest poses cover every node of the document.
        assert_eq!(state.node_rest.len(), gltf.document.nodes().len());
        for (index, rest) in &state.node_rest {
            assert!(!rest.path.is_empty(), "node {index} has no name path");
            assert!(
                rest.path.last().map(Name::as_str).is_some(),
                "a path always ends in a name"
            );
        }
        // VRoid writes a unit-scaled scene root, so dividing the uniform root
        // scale out is the identity here.
        assert_eq!(state.root_scale, 1.0);

        // Expressions, look-at and the `firstPerson` default.
        let vrm = state.vrm.as_ref().expect("VRMC_vrm is present");
        let expressions = vrm
            .expressions
            .as_ref()
            .expect("Elmer declares expressions");
        assert!(expressions.preset.contains_key("happy"));
        assert!(expressions.preset.contains_key("blink"));
        assert!(
            expressions
                .preset
                .values()
                .flat_map(|preset| preset.morph_target_binds.iter().flatten())
                .all(|bind| bind.node < gltf.document.nodes().len() && bind.weight > 0.0),
            "every morph bind names an existing node with a positive weight"
        );
        let look_at = vrm.look_at.as_ref().expect("Elmer declares lookAt");
        assert_eq!(look_at.offset_from_head_bone, [0.0, 0.06, 0.0]);
        assert_eq!(look_at.range_map_horizontal_inner.input_max_value, 90.0);
        // Elmer annotates nothing, which per the specification means every mesh
        // is `auto` -- the case the experiment dropped.
        assert!(
            vrm.first_person
                .as_ref()
                .is_some_and(|first_person| first_person.mesh_annotations.is_empty()),
            "Elmer relies on the `auto` default"
        );

        // Spring bones, with their specification defaults already applied.
        let spring_bone = state.spring_bone.as_ref().unwrap();
        assert_eq!(spring_bone.springs.len(), 40);
        assert_eq!(spring_bone.colliders.len(), 28);
        assert_eq!(spring_bone.collider_groups.len(), 12);
        let joints = spring_bone.all_joints();
        assert!(
            joints
                .iter()
                .all(|joint| joint.stiffness.is_some() && joint.drag_force.is_some()),
            "every joint carries the resolved defaults"
        );

        success!()
    }

    /// The same hook on an avatar that *does* annotate its meshes.
    #[test]
    fn alicia_solid_annotations_are_read() -> TestResult {
        let gltf = avatar("AliciaSolid.vrm");
        let mut state = VrmLoadState::default();

        parse_root_extensions(&mut state, &gltf);

        let first_person = state
            .vrm
            .as_ref()
            .and_then(|vrm| vrm.first_person.as_ref())
            .expect("VRMC_vrm.firstPerson");
        assert_eq!(first_person.mesh_annotations.len(), 12);
        assert!(
            first_person
                .mesh_annotations
                .iter()
                .all(|annotation| annotation.node < gltf.document.nodes().len()),
            "every annotation names an existing node"
        );
        success!()
    }

    /// The smallest glTF document that exercises the rest-pose walk: an unnamed
    /// root node with a uniform scale, one named child, and a deeper grandchild.
    fn two_level_scene() -> String {
        r#"{
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"name": "Armature", "scale": [2.0, 2.0, 2.0], "children": [1]},
                {"name": "Hips", "translation": [0.0, 1.0, 0.0], "children": [2]},
                {"rotation": [0.0, 0.0, 0.0, 1.0]}
            ]
        }"#
        .to_string()
    }

    fn document(json: &str) -> gltf::Gltf {
        // Validation is off: the fixtures below declare meshes without real
        // accessors, which is enough for the node walks under test.
        gltf::Gltf::from_slice_without_validation(json.as_bytes())
            .expect("the test document should parse")
    }

    #[test]
    fn rest_poses_cover_every_node_and_compose_top_down() -> TestResult {
        let gltf = document(&two_level_scene());
        let rests = collect_rest_poses(&gltf);

        assert_eq!(rests.nodes.len(), 3, "every node, not just bones");

        let hips = &rests.nodes[&1];
        assert_eq!(hips.rest, Transform::from_xyz(0.0, 1.0, 0.0));
        // The parent's uniform scale is composed into the world rest …
        assert_eq!(hips.world.translation(), Vec3::new(0.0, 2.0, 0.0));
        // … and is available as the child's parent rotation.
        assert_eq!(hips.parent_world_rotation, Quat::IDENTITY);
        assert_eq!(
            hips.world_translation_without_root_scale(rests.root_scale),
            Vec3::new(0.0, 1.0, 0.0),
            "the file's uniform root scale is divided out on request"
        );

        let leaf = &rests.nodes[&2];
        assert_eq!(leaf.world.translation(), Vec3::new(0.0, 2.0, 0.0));
        assert_eq!(leaf.parent_world_rotation, Quat::IDENTITY);

        success!()
    }

    #[test]
    fn rest_pose_paths_match_bevy_gltf_animation_target_ids() -> TestResult {
        let gltf = document(&two_level_scene());
        let rests = collect_rest_poses(&gltf);

        // `collect_path` (`bevy_gltf/src/loader/gltf_ext/scene.rs`) pushes
        // the topmost node's name first.
        assert_eq!(
            rests.nodes[&1].path,
            vec![Name::new("Armature"), Name::new("Hips")]
        );
        assert_eq!(
            rests.nodes[&1].path_target_id(),
            AnimationTargetId::from_iter(["Armature", "Hips"])
        );
        assert_ne!(
            rests.nodes[&1].path_target_id(),
            super::super::bone_target_id("hips"),
            "the path id and the bone id address the same node differently"
        );

        success!()
    }

    #[test]
    fn unnamed_nodes_are_named_the_way_bevy_gltf_names_them() -> TestResult {
        let gltf = document(
            r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
                "nodes":[{"children":[1]},{"name":"Hips"}]}"#,
        );
        let rests = collect_rest_poses(&gltf);

        assert_eq!(
            rests.nodes[&1].path,
            vec![Name::new("GltfNode0"), Name::new("Hips")],
            "bevy falls back to `GltfNode{{index}}`, so the hashed id must match"
        );

        success!()
    }

    #[test]
    fn a_non_uniform_root_scale_is_reported_and_left_alone() -> TestResult {
        let gltf = document(
            r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
                "nodes":[{"name":"Armature","scale":[2.0,1.0,1.0]}]}"#,
        );

        assert_eq!(collect_rest_poses(&gltf).root_scale, 1.0);
        success!()
    }

    #[test]
    fn mesh_skins_are_indexed_by_the_instancing_node() -> TestResult {
        let gltf = document(
            r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0,1]}],
                "meshes":[{"primitives":[{"attributes":{}}]}],
                "skins":[{"joints":[0]}],
                "nodes":[{"mesh":0,"skin":0},{"mesh":0}]}"#,
        );

        let skins = collect_mesh_skins(&gltf);
        // The second node instances the same mesh without a skin, so there is no
        // single answer and the mesh counts as unskinned instead of taking the
        // first node's answer.
        assert_eq!(skins.get(&0), Some(&None));
        success!()
    }

    #[test]
    fn a_skinned_mesh_reports_its_skin() -> TestResult {
        let gltf = document(
            r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
                "meshes":[{"primitives":[{"attributes":{}}]}],
                "skins":[{"joints":[0]}],
                "nodes":[{"mesh":0,"skin":0}]}"#,
        );

        assert_eq!(collect_mesh_skins(&gltf).get(&0), Some(&Some(0)));
        success!()
    }

    #[test]
    fn parse_root_extensions_reads_a_bare_vrmc_vrm() -> TestResult {
        let gltf = document(
            r#"{"asset":{"version":"2.0"},"scene":0,
                "scenes":[{"nodes":[0]}],
                "nodes":[{"name":"Hips"},{"name":"Head","children":[2]},{"name":"Hair"}],
                "extensionsUsed":["VRMC_vrm"],
                "extensions":{"VRMC_vrm":{
                    "specVersion":"1.0",
                    "humanoid":{"humanBones":{"hips":{"node":0},"head":{"node":1}}},
                    "firstPerson":{"meshAnnotations":[{"node":1,"firstPersonFlag":"auto"}]}
                }}}"#,
        );
        let mut state = VrmLoadState::default();

        parse_root_extensions(&mut state, &gltf);

        assert!(state.is_vrm());
        assert_eq!(state.bone_nodes.get("hips"), Some(&0));
        assert_eq!(state.bone_nodes.get("head"), Some(&1));
        assert_eq!(state.head_subtree, HashSet::from([1, 2]));
        assert!(state.spring_bone.is_none());

        success!()
    }

    /// `VRMC_vrm_animation` is the root of a `.vrma` file. Treating it as a model
    /// root would be a silent data loss: its `expressions` map holds node
    /// references, and `VrmcVrm` accepts them as empty expressions.
    #[test]
    fn vrma_roots_are_not_models() -> TestResult {
        let gltf = document(
            r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
                "nodes":[{"name":"Hips"}],
                "extensionsUsed":["VRMC_vrm_animation"],
                "extensions":{"VRMC_vrm_animation":{
                    "specVersion":"1.0",
                    "humanoid":{"humanBones":{"hips":{"node":0}}},
                    "expressions":{"preset":{"happy":{"node":0}}}
                }}}"#,
        );
        let mut state = VrmLoadState::default();

        parse_root_extensions(&mut state, &gltf);

        assert!(!state.is_vrm(), "a .vrma file is not a model");
        assert!(state.bone_nodes.is_empty());
        success!()
    }

    /// `VRMC_springBone` without `VRMC_vrm` is a prop (or a `.vrma` carrying
    /// spring bones), not an avatar, so the avatar pipeline must stay out of it.
    #[test]
    fn spring_bones_alone_are_not_a_model() -> TestResult {
        let gltf = document(
            r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
                "nodes":[{"name":"Tail"}],
                "extensionsUsed":["VRMC_springBone"],
                "extensions":{"VRMC_springBone":{"specVersion":"1.0"}}}"#,
        );
        let mut state = VrmLoadState::default();

        parse_root_extensions(&mut state, &gltf);

        assert!(state.spring_bone.is_some());
        assert!(!state.is_vrm(), "no `VRMC_vrm`, no avatar");
        success!()
    }
}
