//! Provides support for `VRMC_vrm.firstPerson`.
//!
//! | firstPersonFlag | RenderLayers |
//! |---|---|
//! | `both` | 0 (default) |
//! | `thirdPersonOnly` | `FirstPersonLayers::third_person_only` |
//! | `firstPersonOnly` | `FirstPersonLayers::first_person_only` |
//! | `auto` | mesh is split: head part -> thirdPersonOnly, the rest -> both |

use crate::prelude::ChildSearcher;
use crate::prelude::MToonMaterial;
use crate::vrm::Vrm;
use crate::vrm::gltf::extensions::vrmc_vrm::{FirstPerson, FirstPersonFlag};
use crate::vrm::prelude::HeadBoneEntity;
use bevy::app::{App, Plugin, Update};
use bevy::asset::{Assets, Handle};
use bevy::camera::visibility::{Layer, RenderLayers};
use bevy::ecs::lifecycle::Add;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::{
    component::Component,
    entity::Entity,
    event::EntityEvent,
    hierarchy::{ChildOf, Children},
    name::Name,
    observer::On,
    query::{With, Without},
    resource::Resource,
    system::{Commands, Query, Res, ResMut},
};
use bevy::gltf::GltfNode;
use bevy::mesh::morph::MeshMorphWeights;
use bevy::mesh::{Indices, Mesh, Mesh3d, skinning::SkinnedMesh};
use bevy::pbr::MeshMaterial3d;
use bevy::platform::collections::HashSet;
use bevy::prelude::Deref;
use bevy::reflect::Reflect;

pub(crate) struct VrmFirstPersonPlugin;

impl Plugin for VrmFirstPersonPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.init_resource::<FirstPersonLayers>()
            .register_type::<FirstPersonRegistry>()
            .register_type::<FirstPersonCamera>()
            .register_type::<ThirdPersonCamera>()
            .add_observer(setup_first_person_camera)
            .add_observer(setup_third_person_camera)
            .add_observer(apply_enable_first_person)
            .add_observer(apply_disable_first_person)
            .add_systems(Update, split_auto_meshes);
    }
}

/// Holds `(node name, firstPersonFlag)` pairs from `VRMC_vrm.firstPerson.meshAnnotations`.
#[derive(Component, Deref, Reflect, Default)]
pub struct FirstPersonRegistry(Vec<(Name, FirstPersonFlag)>);

impl FirstPersonRegistry {
    pub fn new(
        first_person: Option<&FirstPerson>,
        node_assets: &Assets<GltfNode>,
        nodes: &[Handle<GltfNode>],
    ) -> Self {
        let Some(fp) = first_person else {
            return Self::default();
        };
        Self(
            fp.mesh_annotations
                .iter()
                .filter_map(|a| {
                    let node = node_assets.get(nodes.get(a.node)?)?;
                    Some((Name::new(node.name.clone()), a.first_person_flag))
                })
                .collect(),
        )
    }
}

/// Render layers used to separate first-person-only and third-person-only meshes.
#[derive(Resource, Clone)]
pub struct FirstPersonLayers {
    pub first_person_only: Layer,
    pub third_person_only: Layer,
}

impl Default for FirstPersonLayers {
    fn default() -> Self {
        Self {
            first_person_only: 7,
            third_person_only: 8,
        }
    }
}

/// Attach to a camera that renders from the avatar's point of view.
#[derive(Component, Debug, Copy, Clone, Reflect)]
#[reflect(Component)]
pub struct FirstPersonCamera;

/// Attach to a camera that observes the avatar from outside.
#[derive(Component, Debug, Copy, Clone, Reflect)]
#[reflect(Component)]
pub struct ThirdPersonCamera;

fn setup_first_person_camera(
    trigger: On<Add, FirstPersonCamera>,
    mut commands: Commands,
    layers: Res<FirstPersonLayers>,
) {
    commands
        .entity(trigger.entity)
        .insert(RenderLayers::default().with(layers.first_person_only));
}

fn setup_third_person_camera(
    trigger: On<Add, ThirdPersonCamera>,
    mut commands: Commands,
    layers: Res<FirstPersonLayers>,
) {
    commands
        .entity(trigger.entity)
        .insert(RenderLayers::default().with(layers.third_person_only));
}

/// Applies first-person render layers to all meshes of the target VRM.
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy_vrm1::prelude::*;
/// fn enable(mut commands: Commands, vrm: Entity) {
///     commands.entity(vrm).trigger(RequestEnableFirstPerson);
/// }
/// ```
#[derive(EntityEvent)]
pub struct RequestEnableFirstPerson(pub Entity);

/// Makes all meshes of the target VRM visible to all cameras again.
#[derive(EntityEvent)]
pub struct RequestDisableFirstPerson(pub Entity);

/// A mesh entity waiting for the `auto` head split.
#[derive(Component)]
pub(crate) struct PendingAutoSplit;

/// Attached to the original mesh entity after the `auto` split.
#[derive(Component)]
pub(crate) struct AutoSplitDone {
    head_entity: Entity,
}

fn apply_enable_first_person(
    trigger: On<RequestEnableFirstPerson>,
    mut commands: Commands,
    registries: Query<&FirstPersonRegistry>,
    searcher: ChildSearcher,
    layers: Res<FirstPersonLayers>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
) {
    let vrm = trigger.event_target();
    let Ok(registry) = registries.get(vrm) else {
        return;
    };

    // Mesh entities already covered by an explicit annotation.
    let mut covered = HashSet::new();
    for (name, flag) in registry.iter() {
        let Some(node) = searcher.find_from_name(vrm, name.as_str()) else {
            continue;
        };
        for mesh_entity in descendants_with_mesh(node, &children, &meshes) {
            covered.insert(mesh_entity);
            apply_flag(&mut commands, mesh_entity, *flag, &layers);
        }
    }

    // Per spec: meshes without an annotation are treated as `auto`.
    for mesh_entity in descendants_with_mesh(vrm, &children, &meshes) {
        if !covered.contains(&mesh_entity) {
            apply_flag(&mut commands, mesh_entity, FirstPersonFlag::Auto, &layers);
        }
    }
}

fn apply_disable_first_person(
    trigger: On<RequestDisableFirstPerson>,
    mut commands: Commands,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
) {
    let vrm = trigger.event_target();
    for mesh_entity in descendants_with_mesh(vrm, &children, &meshes) {
        commands
            .entity(mesh_entity)
            .insert(RenderLayers::default())
            .remove::<PendingAutoSplit>();
    }
}

fn apply_flag(
    commands: &mut Commands,
    mesh_entity: Entity,
    flag: FirstPersonFlag,
    layers: &FirstPersonLayers,
) {
    match flag {
        FirstPersonFlag::Both => {
            commands.entity(mesh_entity).insert(RenderLayers::default());
        }
        FirstPersonFlag::ThirdPersonOnly => {
            commands
                .entity(mesh_entity)
                .insert(RenderLayers::layer(layers.third_person_only));
        }
        FirstPersonFlag::FirstPersonOnly => {
            commands
                .entity(mesh_entity)
                .insert(RenderLayers::layer(layers.first_person_only));
        }
        FirstPersonFlag::Auto => {
            commands.entity(mesh_entity).insert(PendingAutoSplit);
        }
    }
}

fn split_auto_meshes(
    mut commands: Commands,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    already_split: Query<(Entity, &AutoSplitDone), With<PendingAutoSplit>>,
    unskinned: Query<
        Entity,
        (
            With<PendingAutoSplit>,
            Without<SkinnedMesh>,
            Without<AutoSplitDone>,
        ),
    >,
    pending: Query<
        (
            Entity,
            &Mesh3d,
            &SkinnedMesh,
            &ChildOf,
            Option<&MeshMaterial3d<MToonMaterial>>,
            Option<&MeshMorphWeights>,
            Option<&Name>,
        ),
        (With<PendingAutoSplit>, Without<AutoSplitDone>),
    >,
    parents: Query<&ChildOf>,
    vrms: Query<&HeadBoneEntity, With<Vrm>>,
    children: Query<&Children>,
    layers: Res<FirstPersonLayers>,
) {
    // Re-enabling after a previous split: just restore the layers.
    for (entity, done) in already_split.iter() {
        commands
            .entity(entity)
            .insert(RenderLayers::default())
            .remove::<PendingAutoSplit>();
        commands
            .entity(done.head_entity)
            .insert(RenderLayers::layer(layers.third_person_only));
    }

    // Meshes without a skin cannot be weighted to the head bone: treat as `both`.
    for entity in unskinned.iter() {
        commands
            .entity(entity)
            .insert(RenderLayers::default())
            .remove::<PendingAutoSplit>();
    }

    for (entity, mesh3d, skinned, child_of, material, morph_weights, name) in pending.iter() {
        // The VRM root is the ancestor holding `Vrm` + `HeadBoneEntity`.
        // It may not be initialized yet; retry on the next frame.
        let Some(head) = find_head_bone(entity, &parents, &vrms) else {
            continue;
        };

        // The head bone and all of its descendants.
        let mut head_set = HashSet::new();
        collect_descendants(head, &children, &mut head_set);

        // Joint indices of this skin that point into the head subtree.
        let head_joint_ids = skinned
            .joints
            .iter()
            .enumerate()
            .filter(|(_, joint)| head_set.contains(*joint))
            .map(|(index, _)| index as u16)
            .collect::<HashSet<u16>>();

        // The mesh asset may not be loaded yet; retry on the next frame.
        let Some(mesh) = mesh_assets.get(&mesh3d.0) else {
            continue;
        };
        match classify_auto_mesh(mesh, &head_joint_ids) {
            // Nothing is weighted to the head: keep visible everywhere.
            AutoSplit::Both => {
                commands
                    .entity(entity)
                    .insert(RenderLayers::default())
                    .remove::<PendingAutoSplit>();
            }
            // The whole mesh belongs to the head (face, eyes, hair):
            // hide it from first-person cameras, no split needed.
            AutoSplit::ThirdPersonOnly => {
                commands
                    .entity(entity)
                    .insert(RenderLayers::layer(layers.third_person_only))
                    .remove::<PendingAutoSplit>();
            }
            // Mixed weights: split into a head part and the rest.
            AutoSplit::Split(parts) => {
                let (head_mesh, rest_mesh) = *parts;
                let head_handle = mesh_assets.add(head_mesh);
                let rest_handle = mesh_assets.add(rest_mesh);

                // The head part is a new sibling entity under the same glTF node,
                // visible only to third-person cameras.
                let head_name = name.map(Name::as_str).unwrap_or("mesh");
                let mut head_commands = commands.spawn((
                    Name::new(format!("{head_name}.headSplit")),
                    ChildOf(child_of.parent()),
                    Mesh3d(head_handle),
                    skinned.clone(),
                    RenderLayers::layer(layers.third_person_only),
                ));
                if let Some(material) = material {
                    head_commands.insert(material.clone());
                }
                if let Some(morph_weights) = morph_weights {
                    head_commands.insert(morph_weights.clone());
                }
                let head_entity = head_commands.id();

                // The original entity keeps only the non-head part and stays visible everywhere.
                commands
                    .entity(entity)
                    .insert((
                        Mesh3d(rest_handle),
                        RenderLayers::default(),
                        AutoSplitDone { head_entity },
                    ))
                    .remove::<PendingAutoSplit>();
            }
        }
    }
}

/// Walks up the hierarchy to the VRM root and returns its head bone entity.
fn find_head_bone(
    mesh_entity: Entity,
    parents: &Query<&ChildOf>,
    vrms: &Query<&HeadBoneEntity, With<Vrm>>,
) -> Option<Entity> {
    let mut current = mesh_entity;
    loop {
        if let Ok(head) = vrms.get(current) {
            return Some(head.0);
        }
        current = parents.get(current).ok()?.parent();
    }
}

fn collect_descendants(
    entity: Entity,
    children: &Query<&Children>,
    output: &mut HashSet<Entity>,
) {
    output.insert(entity);
    if let Ok(entity_children) = children.get(entity) {
        for child in entity_children {
            collect_descendants(*child, children, output);
        }
    }
}

/// Returns `root` and all of its descendants that have a `Mesh3d`.
fn descendants_with_mesh(
    root: Entity,
    children: &Query<&Children>,
    meshes: &Query<(), With<Mesh3d>>,
) -> Vec<Entity> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if meshes.contains(entity) {
            found.push(entity);
        }
        if let Ok(entity_children) = children.get(entity) {
            stack.extend(entity_children.iter().copied());
        }
    }
    found
}

/// Classification of an `auto` mesh relative to the head bone subtree.
enum AutoSplit {
    /// No triangles are weighted to the head: visible everywhere.
    Both,
    /// Every triangle is weighted to the head (face, eyes, hair):
    /// the whole mesh must be hidden from first-person cameras.
    ThirdPersonOnly,
    /// Mixed weights: (head part, rest part), split by triangle, index-only.
    Split(Box<(Mesh, Mesh)>),
}

fn classify_auto_mesh(
    mesh: &Mesh,
    head_joint_ids: &HashSet<u16>,
) -> AutoSplit {
    use bevy::mesh::VertexAttributeValues;

    if head_joint_ids.is_empty() {
        return AutoSplit::Both;
    }
    let Some(VertexAttributeValues::Uint16x4(joint_indices)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
    else {
        return AutoSplit::Both;
    };
    let Some(VertexAttributeValues::Float32x4(joint_weights)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
    else {
        return AutoSplit::Both;
    };

    let is_head_vertex: Vec<bool> = joint_indices
        .iter()
        .zip(joint_weights)
        .map(|(idx, w)| (0..4).any(|k| w[k] > 0.0 && head_joint_ids.contains(&idx[k])))
        .collect();

    let Some(indices) = mesh.indices() else {
        return AutoSplit::Both;
    };
    let indices: Vec<u32> = indices.iter().map(|i| i as u32).collect();
    let (mut head, mut rest) = (Vec::new(), Vec::new());
    for tri in indices.as_chunks::<3>().0 {
        let target = if tri.iter().any(|&v| is_head_vertex[v as usize]) {
            &mut head
        } else {
            &mut rest
        };
        target.extend_from_slice(tri);
    }

    match (head.is_empty(), rest.is_empty()) {
        (true, _) => AutoSplit::Both,
        (false, true) => AutoSplit::ThirdPersonOnly,
        (false, false) => {
            let mut head_mesh = mesh.clone();
            head_mesh.insert_indices(Indices::U32(head));
            let mut rest_mesh = mesh.clone();
            rest_mesh.insert_indices(Indices::U32(rest));
            AutoSplit::Split(Box::new((head_mesh, rest_mesh)))
        }
    }
}
