//! `on_gltf_primitive`: the `VRMC_vrm.firstPerson` classification and the
//! `auto` split.
//!
//! # What the specification asks for
//!
//! [`VRMC_vrm.firstPerson`](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/firstPerson.md)
//! gives every mesh one of four annotations, and states that **a mesh without an
//! annotation is `auto`**. `auto` means: look at which bones the mesh's vertices
//! are weighted to, hide from a first-person camera whatever hangs off the head
//! bone, and keep the rest.
//!
//! # Why the split happens here
//!
//! A split has to change the `Mesh` asset itself, and `user_mesh` is the only
//! hook that can. The hook is called once per `(mesh, primitive)`, *before*
//! `bevy_gltf` reads the vertices
//! (`crates/bevy_gltf/src/loader/mod.rs`), which is why the joints and
//! weights are read here from the buffer data rather than from a built `Mesh`.
//! The head half is registered as a labeled asset here and spawned as a
//! sibling in [`apply_first_person`](super::scene), because the mesh entities
//! only exist by the time `on_scene_completed` runs.
//!
//! The label is added with the *root* `LoadContext` and read back with the scene
//! one; that works because `LoadContext::begin_labeled_asset`
//! (`bevy_asset/src/loader.rs`) keeps `asset_path` unchanged, so both
//! contexts resolve the same asset path.
//!
//! # Morph targets
//!
//! `user_mesh` replaces the whole mesh, and `bevy_gltf` only reads
//! `morph_targets` in the `else` branch (`loader/mod.rs`) — a handler
//! that supplies a mesh silently loses them. Rather than lose a morph-target
//! mesh's expressions, the morph deltas are read from the same buffers and
//! rebuilt for the compacted vertex list. If that cannot be done exactly, the
//! primitive is **not** split and is classified [`AutoClass::Both`], which is
//! the conservative direction: hiding geometry that turns out to be the body
//! would make an avatar lose its torso.

use bevy::asset::{LoadContext, RenderAssetUsages};
use bevy::gltf::MorphTargetNames;
use bevy::gltf::gltf_ext::mesh::primitive_topology;
use bevy::math::Vec3;
use bevy::mesh::morph::MorphAttributes;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy::platform::collections::{HashMap, HashSet};

use super::VrmLoadState;
use crate::error::vrm_warn;
use crate::vrm::gltf::extensions::vrmc_vrm::{FirstPerson, FirstPersonFlag};

/// Classification/split results keyed by (glTF mesh index, primitive index).
#[derive(Clone, Default)]
pub struct FirstPersonPlan {
    /// How each primitive ended up being classified.
    pub classes: HashMap<(usize, usize), AutoClass>,
    /// Label of the head sub-mesh asset, for the primitives that were split.
    /// The head copy is spawned in [`apply_first_person`](super::scene).
    pub head_mesh_labels: HashMap<(usize, usize), String>,
    /// The glTF mesh indices `auto` applies to, computed on the first primitive.
    auto_meshes: Option<HashSet<usize>>,
}

/// What `firstPerson` decided about one primitive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoClass {
    /// Visible in both views.
    Both,
    /// Entirely on head bones: hidden from a first-person camera.
    ThirdPersonOnly,
    /// Mixed: split into a body part (both) and a head part (third-person only).
    Split,
}

pub(crate) fn process_primitive(
    state: &mut VrmLoadState,
    load_context: &mut LoadContext<'_>,
    gltf_document: &gltf::Gltf,
    gltf_mesh: &gltf::Mesh,
    gltf_primitive: &gltf::Primitive,
    buffer_data: &[Vec<u8>],
    user_mesh: &mut Option<Mesh>,
) {
    // An unannotated mesh is `auto`, so only the head subtree can stop the work
    // here: without it every mesh stays `both`.
    if state.head_subtree.is_empty() {
        return;
    }
    if !is_auto(state, gltf_document, gltf_mesh.index()) {
        return;
    }

    let key = (gltf_mesh.index(), gltf_primitive.index());
    let Some(class) = classify(
        state,
        gltf_document,
        gltf_mesh.index(),
        gltf_primitive,
        buffer_data,
    ) else {
        return;
    };
    state.first_person.classes.insert(key, class);
    if class != AutoClass::Split {
        return;
    }

    let Some(parts) = split(state, gltf_document, gltf_mesh, gltf_primitive, buffer_data) else {
        // The classification was `Split` but the geometry could not be divided.
        // Falling back to `Both` keeps the face visible in both views, which is
        // the conservative direction: hiding geometry that turns out to be the
        // body would make an avatar lose its torso.
        state.first_person.classes.insert(key, AutoClass::Both);
        return;
    };
    let (body, head) = parts;

    // `classify` decides per *vertex*, but `build_submesh` sends a whole triangle
    // to the head as soon as any of its vertices is a head vertex. The two can
    // disagree and leave one half empty. An empty mesh must never be added as an
    // asset: bevy's mesh allocator extracts it, skips the zero-length allocation,
    // then tries to copy into the missing slot and logs a use-after-free
    // (`bevy_render::slab_allocator`).
    let body_vertices = body.count_vertices();
    let head_vertices = head.count_vertices();
    if body_vertices == 0 || head_vertices == 0 {
        // Keep bevy's own mesh and classify the primitive as a whole: no head
        // triangle means it is visible in both views, no body triangle means it
        // is head-only.
        let class = if head_vertices == 0 {
            AutoClass::Both
        } else {
            AutoClass::ThirdPersonOnly
        };
        state.first_person.classes.insert(key, class);
        return;
    }

    // The body half replaces the primitive bevy is about to build.
    *user_mesh = Some(body);

    let label = format!("Mesh{}/Primitive{}/VrmHeadOnly", key.0, key.1);
    load_context.add_labeled_asset::<Mesh>(label.clone(), head);
    state.first_person.head_mesh_labels.insert(key, label);
}

/// Whether a node instancing this mesh is `auto`.
fn is_auto(
    state: &mut VrmLoadState,
    gltf_document: &gltf::Gltf,
    mesh_index: usize,
) -> bool {
    let Some(vrm) = &state.vrm else {
        return false;
    };
    state
        .first_person
        .auto_meshes
        .get_or_insert_with(|| auto_meshes(vrm.first_person.as_ref(), &gltf_document.document))
        .contains(&mesh_index)
}

/// The meshes instanced by an `auto` node.
///
/// A node takes the flag of the nearest annotated node among itself and its
/// ancestors, so an annotation on a mesh-less parent covers the meshes below it.
/// A node no annotation covers is `auto`, which is also what a file without
/// `firstPerson` gets.
fn auto_meshes(
    first_person: Option<&FirstPerson>,
    document: &gltf::Document,
) -> HashSet<usize> {
    let flags: HashMap<usize, FirstPersonFlag> = first_person
        .map(|first_person| {
            first_person
                .mesh_annotations
                .iter()
                .map(|annotation| (annotation.node, annotation.first_person_flag))
                .collect()
        })
        .unwrap_or_default();
    let parents: HashMap<usize, usize> = document
        .nodes()
        .flat_map(|node| {
            node.children()
                .map(move |child| (child.index(), node.index()))
        })
        .collect();
    document
        .nodes()
        .filter_map(|node| {
            let mesh = node.mesh()?;
            let flag =
                std::iter::successors(Some(node.index()), |index| parents.get(index).copied())
                    .find_map(|index| flags.get(&index).copied())
                    .unwrap_or(FirstPersonFlag::Auto);
            (flag == FirstPersonFlag::Auto).then_some(mesh.index())
        })
        .collect()
}

/// Splits the primitive's vertices by which triangles are weighted to the head
/// subtree.
///
/// `None` when the classification cannot be made — no skin, no joints, or a
/// joint format the mesh cannot be read from — in which case the primitive keeps
/// bevy's own mesh and stays visible in both views.
fn classify(
    state: &VrmLoadState,
    gltf_document: &gltf::Gltf,
    mesh_index: usize,
    gltf_primitive: &gltf::Primitive,
    buffer_data: &[Vec<u8>],
) -> Option<AutoClass> {
    let head_slots = head_joint_slots(state, gltf_document, mesh_index)?;
    if head_slots.is_empty() {
        return Some(AutoClass::Both);
    }
    let (joints, weights) = read_skin(gltf_primitive, buffer_data)?;
    let head_vertex = head_vertex_flags(&joints, &weights, &head_slots);
    Some(
        match head_vertex.iter().filter(|is_head| **is_head).count() {
            0 => AutoClass::Both,
            count if count == head_vertex.len() => AutoClass::ThirdPersonOnly,
            _ => AutoClass::Split,
        },
    )
}

/// The joint slots of the mesh's skin that point into the head subtree.
///
/// The skin is indexed by **mesh index**, not looked up from "the first node
/// that references this mesh": see
/// [`collect_mesh_skins`](super::root::collect_mesh_skins) for why the hook
/// cannot see the entity's own node and how an ambiguous mesh is handled.
fn head_joint_slots(
    state: &VrmLoadState,
    gltf_document: &gltf::Gltf,
    mesh_index: usize,
) -> Option<HashSet<u16>> {
    let skin_index = state.mesh_skins.get(&mesh_index).copied().flatten()?;
    let skin = gltf_document.document.skins().nth(skin_index)?;
    Some(
        skin.joints()
            .enumerate()
            .filter(|(_, joint)| state.head_subtree.contains(&joint.index()))
            .map(|(slot, _)| slot as u16)
            .collect(),
    )
}

/// `JOINTS_0` and `WEIGHTS_0` as `u16` joint slots and `f32` weights.
///
/// `into_u16` widens the glTF-permitted `UNSIGNED_BYTE` layout as well, so a
/// file that stores `JOINTS_0` compactly is classified like any other instead of
/// falling back to `Both`.
fn read_skin<'r>(
    gltf_primitive: &gltf::Primitive<'r>,
    buffer_data: &[Vec<u8>],
) -> Option<(Vec<[u16; 4]>, Vec<[f32; 4]>)> {
    let reader = gltf_primitive.reader(|buffer| buffer_data.get(buffer.index()).map(Vec::as_slice));
    let (Some(joints), Some(weights)) = (reader.read_joints(0), reader.read_weights(0)) else {
        return None;
    };
    let joints: Vec<[u16; 4]> = joints.into_u16().collect();
    let weights: Vec<[f32; 4]> = weights.into_f32().collect();
    if joints.len() != weights.len() {
        vrm_warn!(format!(
            "VRM firstPerson: JOINTS_0 has {} entries and WEIGHTS_0 has {}; treating the mesh as \
             visible in both views",
            joints.len(),
            weights.len()
        ));
        return None;
    }
    Some((joints, weights))
}

/// The per-vertex head flag, computed the same way for [`classify`] and
/// [`split`] so the two never disagree.
fn head_vertex_flags(
    joints: &[[u16; 4]],
    weights: &[[f32; 4]],
    head_slots: &HashSet<u16>,
) -> Vec<bool> {
    joints
        .iter()
        .zip(weights)
        .map(|(joint, weight)| is_head_vertex(joint, weight, head_slots))
        .collect()
}

/// Whether the head subtree holds the **majority** of this vertex's influence.
///
/// The specification (`firstPerson.md`, "MeshAnnotation.Auto Algorithm") only
/// says a vertex "contains the weight of Head bone", names no threshold and
/// prescribes no method for the split, so a threshold is the library's choice;
/// the majority rule is admissible and is chosen to match the algorithm's
/// stated intent — hide what blocks the first-person view (the face, head and
/// hair), not the neck. Reading "contains" as "any weight above zero" swallows
/// the neck of a model whose neck skin blends toward the head
/// (`ANIMA_MA_PATCH.md`, patch 2): a `VRoid` 1.x export carries a smooth
/// head-weight gradient (measured 0.03 at the throat up to 0.87 under the jaw),
/// so a throat triangle whose neck joint dominates is classified head-side,
/// moved to the third-person-only layer and disappears from the first-person
/// view. Requiring the head subtree to hold more than half of the vertex's
/// total influence puts the split at the anatomical head on every model, and
/// leaves a dedicated neck mesh weighted to the neck bone alone visible.
fn is_head_vertex(
    joint: &[u16; 4],
    weight: &[f32; 4],
    head_slots: &HashSet<u16>,
) -> bool {
    let total: f32 = weight.iter().sum();
    let head: f32 = (0..4)
        .filter(|&k| head_slots.contains(&joint[k]))
        .map(|k| weight[k])
        .sum();
    head * 2.0 > total
}

/// Per-vertex data of one primitive, plus the per-vertex head flag.
struct PrimitiveData<'a> {
    topology: PrimitiveTopology,
    positions: &'a [[f32; 3]],
    normals: Option<&'a [[f32; 3]]>,
    tex_coords_0: Option<&'a [[f32; 2]]>,
    tex_coords_1: Option<&'a [[f32; 2]]>,
    colors: Option<&'a [[f32; 4]]>,
    tangents: Option<&'a [[f32; 4]]>,
    joints: &'a [[u16; 4]],
    weights: &'a [[f32; 4]],
    morph_targets: Vec<(Option<Vec<Vec3>>, Option<Vec<Vec3>>, Option<Vec<Vec3>>)>,
    indices: Vec<u32>,
    head_vertex: Vec<bool>,
}

/// Reads the primitive and divides it into `(body, head)`.
///
/// Both halves keep the source topology, every attribute the source carried, and
/// the morph targets; only the index list and the vertex set shrink, which is
/// what makes the split lossless.
fn split(
    state: &VrmLoadState,
    gltf_document: &gltf::Gltf,
    gltf_mesh: &gltf::Mesh,
    gltf_primitive: &gltf::Primitive,
    buffer_data: &[Vec<u8>],
) -> Option<(Mesh, Mesh)> {
    let topology = primitive_topology(gltf_primitive.mode()).ok()?;
    let head_slots = head_joint_slots(state, gltf_document, gltf_mesh.index())?;
    let (joints, weights) = read_skin(gltf_primitive, buffer_data)?;

    let reader = gltf_primitive.reader(|buffer| buffer_data.get(buffer.index()).map(Vec::as_slice));
    let positions: Vec<[f32; 3]> = reader.read_positions()?.collect();
    let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|values| values.collect());
    let tex_coords_0: Option<Vec<[f32; 2]>> =
        reader.read_tex_coords(0).map(|uv| uv.into_f32().collect());
    let tex_coords_1: Option<Vec<[f32; 2]>> =
        reader.read_tex_coords(1).map(|uv| uv.into_f32().collect());
    let colors: Option<Vec<[f32; 4]>> = reader
        .read_colors(0)
        .map(|values| values.into_rgba_f32().collect());
    let tangents: Option<Vec<[f32; 4]>> = reader.read_tangents().map(|values| values.collect());
    let indices: Vec<u32> = match reader.read_indices() {
        Some(read) => read.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };
    // Morph deltas are per-vertex, exactly like the base attributes.
    let morph_targets: Vec<_> = reader
        .read_morph_targets()
        .map(|(positions, normals, tangents)| {
            (
                positions.map(|d| d.map(Vec3::from).collect::<Vec<_>>()),
                normals.map(|d| d.map(Vec3::from).collect::<Vec<_>>()),
                tangents.map(|d| d.map(Vec3::from).collect::<Vec<_>>()),
            )
        })
        .collect();

    let head_vertex = head_vertex_flags(&joints, &weights, &head_slots);

    let data = PrimitiveData {
        topology,
        positions: &positions,
        normals: normals.as_deref(),
        tex_coords_0: tex_coords_0.as_deref(),
        tex_coords_1: tex_coords_1.as_deref(),
        colors: colors.as_deref(),
        tangents: tangents.as_deref(),
        joints: &joints,
        weights: &weights,
        morph_targets,
        indices,
        head_vertex,
    };

    let mut body = build_submesh(&data, false)?;
    let mut head = build_submesh(&data, true)?;

    // What `bevy_gltf` does for the meshes it builds itself.
    if !data.morph_targets.is_empty()
        && let Some(names) = gltf_mesh
            .extras()
            .as_ref()
            .and_then(|extras| serde_json::from_str::<MorphTargetNames>(extras.get()).ok())
    {
        body.set_morph_target_names(names.target_names.clone());
        head.set_morph_target_names(names.target_names);
    }
    // `bevy_gltf` generates these for the body half, which it receives as
    // `user_mesh`, but never sees the head half. Without them the copy's `Aabb`
    // stays at the bind pose.
    if let Err(error) = head.generate_skinned_mesh_bounds() {
        vrm_warn!(format!(
            "VRM firstPerson: failed to generate the head half's skinned mesh bounds: {error}"
        ));
    }

    Some((body, head))
}

/// One source vertex, gathered for the compacted vertex list.
///
/// The channels the primitive does not carry stay `None` and are not written to
/// the sub-mesh, so the split never invents an attribute. A channel that *is*
/// carried but shorter than the position array is a malformed accessor: that is
/// reported by the `None` from [`compact_vertex`], never by silently padding.
struct CompactedVertex {
    mapped: u32,
    position: [f32; 3],
    normals: Option<[f32; 3]>,
    tex_coord_0: Option<[f32; 2]>,
    tex_coord_1: Option<[f32; 2]>,
    color: Option<[f32; 4]>,
    tangent: Option<[f32; 4]>,
    joint: [u16; 4],
    weight: [f32; 4],
}

/// Copies one source vertex into the compacted lists and records its new index.
fn compact_vertex(
    data: &PrimitiveData<'_>,
    index: u32,
    sources: &mut Vec<usize>,
    remap: &mut HashMap<u32, u32>,
) -> Option<CompactedVertex> {
    let source = index as usize;
    let mapped = sources.len() as u32;
    remap.insert(index, mapped);
    sources.push(source);
    Some(CompactedVertex {
        mapped,
        position: *data.positions.get(source)?,
        normals: element(data.normals, source)?,
        tex_coord_0: element(data.tex_coords_0, source)?,
        tex_coord_1: element(data.tex_coords_1, source)?,
        color: element(data.colors, source)?,
        tangent: element(data.tangents, source)?,
        joint: *data.joints.get(source)?,
        weight: *data.weights.get(source)?,
    })
}

/// One element of an optional vertex channel.
///
/// `Some(None)` is "the primitive does not carry this channel", `Some(Some(v))`
/// is the element, and `None` is "the channel is there but too short", i.e.
/// malformed exporter output. Collapsing the last case into the first would let
/// a broken attribute array through as a *missing* one.
fn element<T: Copy>(
    values: Option<&[T]>,
    index: usize,
) -> Option<Option<T>> {
    match values {
        None => Some(None),
        Some(values) => values.get(index).copied().map(Some),
    }
}

/// Partitions the triangles by head membership and compacts the vertices each
/// half actually uses.
///
/// Returns `None` when the primitive has fewer than three indices per triangle,
/// or an attribute array that disagrees with the positions — the caller then
/// leaves the primitive unsplit.
fn build_submesh(
    data: &PrimitiveData<'_>,
    keep_head: bool,
) -> Option<Mesh> {
    // Triangles are consumed three at a time. Anything else (a line list, a
    // point cloud) has no such notion, so it is not divided.
    if !data.indices.len().is_multiple_of(3) {
        return None;
    }

    // Source vertex -> destination vertex, and the source indices in output
    // order (needed to rebuild the per-vertex morph deltas).
    let mut remap: HashMap<u32, u32> = HashMap::default();
    let mut sources: Vec<usize> = Vec::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut tex_coords_0 = Vec::new();
    let mut tex_coords_1 = Vec::new();
    let mut colors = Vec::new();
    let mut tangents = Vec::new();
    let mut joints = Vec::new();
    let mut weights = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for triangle in data.indices.as_chunks::<3>().0 {
        let is_head = triangle.iter().any(|index| {
            data.head_vertex
                .get(*index as usize)
                .copied()
                .unwrap_or(false)
        });
        if is_head != keep_head {
            continue;
        }
        for &index in triangle {
            if let Some(&mapped) = remap.get(&index) {
                indices.push(mapped);
                continue;
            }
            // A vertex index with no position - or a shorter attribute array than
            // the position one - is a malformed accessor. The whole split is
            // refused rather than half-applied.
            let compacted = compact_vertex(data, index, &mut sources, &mut remap)?;
            positions.push(compacted.position);
            if let Some(value) = compacted.normals {
                normals.push(value);
            }
            if let Some(value) = compacted.tex_coord_0 {
                tex_coords_0.push(value);
            }
            if let Some(value) = compacted.tex_coord_1 {
                tex_coords_1.push(value);
            }
            if let Some(value) = compacted.color {
                colors.push(value);
            }
            if let Some(value) = compacted.tangent {
                tangents.push(value);
            }
            joints.push(compacted.joint);
            weights.push(compacted.weight);
            indices.push(compacted.mapped);
        }
    }

    let mut mesh = Mesh::new(
        data.topology,
        // `on_gltf_primitive` does not receive the loader settings, and
        // `RenderAssetUsages::default()` is exactly
        // `GltfLoaderSettings::load_meshes`'s default
        // (`bevy_gltf/src/loader/mod.rs`).
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    if !normals.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    }
    if !tex_coords_0.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, tex_coords_0);
    }
    if !tex_coords_1.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, tex_coords_1);
    }
    if !colors.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    if !tangents.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    }
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_INDEX,
        VertexAttributeValues::Uint16x4(joints),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    if !indices.is_empty() {
        mesh.insert_indices(Indices::U32(indices));
    }

    if !data.morph_targets.is_empty() {
        // The morph buffer is weight-major:
        // `morph_targets[target * vertex_count + vertex]`
        // (`crates/bevy_pbr/src/render/morph.wgsl`, `get_morph_target`), which
        // is the order `bevy_gltf` produces by walking the vertices of one
        // target at a time (`crates/bevy_gltf/src/loader/mod.rs`).
        let vertex_count = sources.len();
        let mut rebuilt = Vec::with_capacity(data.morph_targets.len() * vertex_count);
        for (position, normal, tangent) in &data.morph_targets {
            for source in &sources {
                rebuilt.push(MorphAttributes::new(
                    component(position, *source),
                    component(normal, *source),
                    component(tangent, *source),
                ));
            }
        }
        mesh.set_morph_targets(rebuilt);
    }

    Some(mesh)
}

/// One component of one vertex's morph-target displacement.
///
/// A channel the target does not declare reads as zero, exactly as
/// `bevy_gltf`'s `PrimitiveMorphAttributesIter::next` fills it.
fn component(
    source: &Option<Vec<Vec3>>,
    index: usize,
) -> Vec3 {
    source
        .as_ref()
        .and_then(|values| values.get(index))
        .copied()
        .unwrap_or(Vec3::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::TestResult;

    /// Two triangles: `0,1,2` weighted to joint 0 and `3,4,5` weighted to joint
    /// 1, with normals, a uv set and one morph target on top.
    ///
    /// `PrimitiveData` borrows, so the owned vectors live in the fixture and the
    /// borrowed view is handed out by [`Fixture::primitive`].
    struct Fixture {
        positions: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        tex_coords_0: Vec<[f32; 2]>,
        joints: Vec<[u16; 4]>,
        weights: Vec<[f32; 4]>,
        morph_targets: Vec<(Option<Vec<Vec3>>, Option<Vec<Vec3>>, Option<Vec<Vec3>>)>,
        /// Truncated normals, used by the malformed-input test.
        short_normals: bool,
    }

    impl Fixture {
        fn new() -> Self {
            let positions: Vec<[f32; 3]> = (0..6).map(|i| [i as f32, 0.0, 0.0]).collect();
            let joints: Vec<[u16; 4]> = vec![
                [0, 0, 0, 0],
                [0, 0, 0, 0],
                [0, 0, 0, 0],
                [1, 0, 0, 0],
                [1, 0, 0, 0],
                [1, 0, 0, 0],
            ];
            let weights: Vec<[f32; 4]> = joints
                .iter()
                .map(|joint| {
                    let mut weight = [0.0; 4];
                    weight[joint[0] as usize] = 1.0;
                    weight
                })
                .collect();
            Self {
                normals: positions.iter().map(|_| [0.0, 1.0, 0.0]).collect(),
                tex_coords_0: positions.iter().map(|p| [p[0], 0.0]).collect(),
                positions,
                joints,
                weights,
                morph_targets: vec![(Some(vec![Vec3::X; 6]), Some(vec![Vec3::Y; 6]), None)],
                short_normals: false,
            }
        }

        fn primitive(&self) -> PrimitiveData<'_> {
            let normals: &[[f32; 3]] = if self.short_normals {
                &self.normals[..2]
            } else {
                &self.normals
            };
            PrimitiveData {
                // A strip, not a triangle list: the experiment hardcoded
                // `TriangleList`.
                topology: PrimitiveTopology::TriangleStrip,
                positions: &self.positions,
                normals: Some(normals),
                tex_coords_0: Some(&self.tex_coords_0),
                tex_coords_1: None,
                colors: None,
                tangents: None,
                joints: &self.joints,
                weights: &self.weights,
                morph_targets: self.morph_targets.clone(),
                indices: vec![0, 1, 2, 3, 4, 5],
                head_vertex: vec![false, false, false, true, true, true],
            }
        }
    }

    fn indices_of(mesh: &Mesh) -> Vec<u32> {
        mesh.indices()
            .map(|indices| indices.iter().map(|index| index as u32).collect())
            .unwrap_or_default()
    }

    fn first_joint(mesh: &Mesh) -> u16 {
        match mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX) {
            Some(VertexAttributeValues::Uint16x4(rows)) => rows[0][0],
            _ => panic!("the joint indices are kept as `Uint16x4`"),
        }
    }

    /// The split is lossless: both halves keep the source topology, every
    /// attribute the source had, and the morph targets - only the vertex set and
    /// the index list shrink.
    #[test]
    fn the_split_keeps_topology_attributes_and_morph_targets() -> TestResult {
        let fixture = Fixture::new();
        let data = fixture.primitive();

        let body = build_submesh(&data, false).expect("the body half splits");
        let head = build_submesh(&data, true).expect("the head half splits");

        for mesh in [&body, &head] {
            assert_eq!(mesh.primitive_topology(), PrimitiveTopology::TriangleStrip);
            assert_eq!(
                mesh.count_vertices(),
                3,
                "each half keeps only the vertices it uses"
            );
            assert_eq!(indices_of(mesh), vec![0, 1, 2]);
            for attribute in [
                Mesh::ATTRIBUTE_POSITION,
                Mesh::ATTRIBUTE_NORMAL,
                Mesh::ATTRIBUTE_UV_0,
                Mesh::ATTRIBUTE_JOINT_INDEX,
                Mesh::ATTRIBUTE_JOINT_WEIGHT,
            ] {
                assert_eq!(
                    mesh.attribute(attribute).map(|values| values.len()),
                    Some(3),
                    "`{attribute:?}` is carried over"
                );
            }
            // The morph buffer is weight-major and one entry per vertex, so one
            // target over three vertices is three entries.
            let targets = mesh
                .morph_targets()
                .expect("the morph targets are carried over");
            assert_eq!(targets.len(), 3);
            assert_eq!(targets[0].position, Vec3::X);
            assert_eq!(targets[0].normal, Vec3::Y);
            assert_eq!(targets[0].tangent, Vec3::ZERO);
        }

        // The joint weights decide which half a triangle belongs to.
        assert_eq!(first_joint(&body), 0);
        assert_eq!(first_joint(&head), 1);
        success!()
    }

    /// An attribute that is shorter than the position array is malformed
    /// exporter output; the split is refused rather than half-applied, which is
    /// what makes the caller fall back to `Both`.
    #[test]
    fn a_short_attribute_array_refuses_the_split() -> TestResult {
        let mut fixture = Fixture::new();
        fixture.short_normals = true;
        let data = fixture.primitive();

        assert!(build_submesh(&data, false).is_none());
        assert!(build_submesh(&data, true).is_none());
        success!()
    }

    /// An index list that is not made of whole triangles is not divided at all.
    #[test]
    fn an_index_list_that_is_not_triangles_refuses_the_split() -> TestResult {
        let mut fixture = Fixture::new();
        let mut data = fixture.primitive();
        // A borrow-safe copy is not needed: the fixture outlives `data`, so only
        // the index list is replaced.
        let fixture2 = Fixture::new();
        let mut data2 = fixture2.primitive();
        data2.indices = vec![0, 1];
        drop(data);

        assert!(build_submesh(&data2, false).is_none());
        // The fixture is still usable, so the test does not leak a borrow.
        assert_eq!(fixture.morph_targets.len(), 1);
        success!()
    }

    /// `classify` counts head vertices one vertex at a time, but `build_submesh`
    /// sends a whole triangle to the head as soon as *any* of its vertices is a
    /// head vertex. When every triangle touches a head vertex the body half comes
    /// out empty, and adding that empty mesh as an asset once made bevy's mesh
    /// allocator log a use-after-free. This pins the premise of the guard in
    /// `process_primitive`.
    #[test]
    fn an_all_head_primitive_has_an_empty_body_half() -> TestResult {
        let fixture = Fixture::new();
        let mut data = fixture.primitive();
        data.head_vertex = vec![true; 6];

        let body = build_submesh(&data, false).expect("the topology is a strip");
        assert_eq!(body.count_vertices(), 0, "the body half is empty");
        let head = build_submesh(&data, true).expect("the topology is a strip");
        assert_eq!(head.count_vertices(), 6, "the head half keeps every vertex");
        success!()
    }

    fn avatar(name: &str) -> gltf::Gltf {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/vrm")
            .join(name);
        let bytes = std::fs::read(path).expect("the avatar should be readable");
        gltf::Gltf::from_slice_without_validation(&bytes).expect("the avatar should parse")
    }

    /// `root -> {Face (mesh 0), HairRoot -> Hair (mesh 1), Body (mesh 2)}`,
    /// and the meshes `is_auto` accepts under `annotations`.
    fn auto_meshes_under(annotations: &str) -> Vec<usize> {
        let vrm = serde_json::from_str(&format!(
            r#"{{
                "specVersion": "1.0",
                "humanoid": {{"humanBones": {{}}}},
                "firstPerson": {{"meshAnnotations": {annotations}}}
            }}"#
        ))
        .expect("the extension should parse");
        let mut state = VrmLoadState {
            vrm: Some(std::sync::Arc::new(vrm)),
            ..Default::default()
        };
        let gltf = gltf::Gltf::from_slice_without_validation(
            br#"{
                "asset": {"version": "2.0"},
                "scenes": [{"nodes": [0]}],
                "nodes": [
                    {"name": "root", "children": [1, 2, 3]},
                    {"name": "Face", "mesh": 0},
                    {"name": "HairRoot", "children": [4]},
                    {"name": "Body", "mesh": 2},
                    {"name": "Hair", "mesh": 1}
                ],
                "meshes": [
                    {"primitives": [{"attributes": {}}]},
                    {"primitives": [{"attributes": {}}]},
                    {"primitives": [{"attributes": {}}]}
                ]
            }"#,
        )
        .expect("the document should parse");
        (0..3)
            .filter(|mesh| is_auto(&mut state, &gltf, *mesh))
            .collect()
    }

    /// The specification makes every mesh no annotation covers `auto`, not only
    /// the meshes of a file that annotates nothing.
    #[test]
    fn an_unannotated_mesh_is_auto_next_to_annotated_ones() -> TestResult {
        assert_eq!(
            auto_meshes_under(r#"[{"node": 3, "firstPersonFlag": "both"}]"#),
            vec![0, 1]
        );
        success!()
    }

    /// An annotation on a mesh-less node covers the meshes below it, and the
    /// nearest annotation wins.
    #[test]
    fn an_annotation_reaches_the_meshes_below_its_node() -> TestResult {
        assert_eq!(
            auto_meshes_under(r#"[{"node": 2, "firstPersonFlag": "thirdPersonOnly"}]"#),
            vec![0, 2]
        );
        assert_eq!(
            auto_meshes_under(
                r#"[
                    {"node": 0, "firstPersonFlag": "both"},
                    {"node": 2, "firstPersonFlag": "auto"}
                ]"#
            ),
            vec![1]
        );
        success!()
    }

    /// The head half is registered by the handler rather than by `bevy_gltf`, so
    /// it has to carry what `bevy_gltf` would have given it: the skinned bounds
    /// that make its `Aabb` follow the pose, and the morph target names.
    #[test]
    fn a_split_keeps_skinned_bounds_and_target_names() -> TestResult {
        let gltf = avatar("AliciaSolid.vrm");
        let buffer_data = vec![gltf.blob.clone().expect("a .vrm is a GLB")];
        let mut state = VrmLoadState::default();
        crate::vrm::gltf::handler::root::parse_root_extensions(&mut state, &gltf);

        let (mesh, primitive) = gltf
            .document
            .meshes()
            .flat_map(|mesh| {
                mesh.primitives()
                    .map(move |primitive| (mesh.clone(), primitive))
            })
            .find(|(mesh, primitive)| {
                primitive.morph_targets().len() > 0
                    && classify(&state, &gltf, mesh.index(), primitive, &buffer_data)
                        == Some(AutoClass::Split)
            })
            .expect("AliciaSolid's face is split");
        let names = mesh
            .extras()
            .as_ref()
            .and_then(|extras| serde_json::from_str::<MorphTargetNames>(extras.get()).ok())
            .expect("the face names its morph targets")
            .target_names;

        let (body, head) =
            split(&state, &gltf, &mesh, &primitive, &buffer_data).expect("the face splits");

        assert_eq!(body.morph_target_names(), Some(names.as_slice()));
        assert_eq!(head.morph_target_names(), Some(names.as_slice()));
        assert!(head.skinned_mesh_bounds().is_some());
        success!()
    }

    /// The `ANIMA_MA_PATCH.md` majority rule: a vertex the head subtree only
    /// partly influences (the throat on a model whose neck skin blends toward
    /// the head) is not head-side, so its geometry stays visible in first
    /// person. Reading the rule as "any head weight at all" hid the neck of
    /// every `VRoid` export except the models whose neck is a dedicated mesh.
    #[test]
    fn a_neck_dominant_vertex_stays_visible() -> TestResult {
        let head_slots: HashSet<u16> = [1u16].into_iter().collect();
        // Slot 1 is the head joint, slot 0 the neck. v0 is the throat (0.3 head
        // / 0.7 neck), v1 the jaw (0.8 head / 0.2 neck).
        let joints = [[1, 0, 0, 0], [1, 0, 0, 0]];
        let weights = [[0.3, 0.7, 0.0, 0.0], [0.8, 0.2, 0.0, 0.0]];
        assert_eq!(
            head_vertex_flags(&joints, &weights, &head_slots),
            vec![false, true],
            "only the head-dominant vertex is head-side; the throat stays visible"
        );
        success!()
    }
}
