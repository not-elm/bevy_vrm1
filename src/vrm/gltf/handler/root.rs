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
        if let Some(head) = vrm.humanoid.human_bones.get(HEAD_BONE) {
            state.head_subtree = collect_subtree(gltf, head.node);
        }
    }

    state.mesh_skins = collect_mesh_skins(gltf);
    state.texture_count = gltf.textures().len();
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

        let skinned = state
            .mesh_skins
            .values()
            .filter(|skin| skin.is_some())
            .count();
        assert_eq!(skinned, 3, "all three of Elmer's meshes are skinned");

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
        assert!(
            vrm.first_person
                .as_ref()
                .is_some_and(|first_person| first_person.mesh_annotations.is_empty()),
            "Elmer relies on the `auto` default"
        );

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

    fn document(json: &str) -> gltf::Gltf {
        gltf::Gltf::from_slice_without_validation(json.as_bytes())
            .expect("the test document should parse")
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
