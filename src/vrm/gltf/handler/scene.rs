//! `on_scene_completed`: the whole VRM initialization, run once per glTF scene
//! before the scratch world is frozen into a
//! [`WorldAsset`](bevy::world_serialization::WorldAsset).
//!
//! `bevy_gltf` calls this hook in `crates/bevy_gltf/src/loader/mod.rs`
//! and only then runs `WorldAsset::new(world)`, so everything written
//! here lands in the asset and is copied — through reflection, entity fields
//! remapped — into every `SceneRoot` instance of it.
//!
//! # Order
//!
//! 1. rest poses, because the spring-bone joint state is derived from them;
//! 2. spring-bone chains and colliders;
//! 3. node constraints: node indices resolved to entities and sorted;
//! 4. expressions and look-at on the scene root;
//! 5. animation targets and the player;
//! 6. the bone markers and the `<Bone>BoneEntity` holders on the scene root,
//!    without which gaze control and body tracking match nothing;
//! 7. the source asset path, then the firstPerson views and the head copies;
//! 8. the private `VrmSceneRoot` marker; `Vrm` + `Initialized` are inserted by
//!    the attach observer once the root data has been moved onto the
//!    application's entity (see [`crate::vrm::instance`]).

use bevy::animation::{AnimatedBy, AnimationPlayer};
use bevy::asset::{Handle, LoadContext};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::{DynamicSkinnedMeshBounds, NoFrustumCulling};
use bevy::ecs::entity::Entity;
use bevy::ecs::world::World;
use bevy::mesh::morph::{MeshMorphWeights, MorphWeights};
use bevy::mesh::skinning::SkinnedMesh;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

use super::first_person::AutoClass;
use super::{VrmLoadState, bone_target_id};
use crate::error::vrm_warn;
use crate::prelude::{
    ConstraintExecutionOrder, EffectiveExpressionWeights, ExpressionMorphBinds, ExpressionSetting,
    ExpressionSettings, MToonMaterial, MorphBind, PendingNodeConstraint, RestGlobalTransform,
    RestTransform, RestWorldTransform, Vrm, VrmBone, VrmExpressionWeights, VrmHeadOnly,
    VrmNodeConstraint, VrmPath, vrm_root_animation_target,
};
use crate::vrm::components::FirstPersonView;
use crate::vrm::expressions::{ExpressionCategory, ExpressionOverrideType};
use crate::vrm::gltf::extensions::vrmc_spring_bone::ColliderShape;
use crate::vrm::gltf::extensions::vrmc_vrm::FirstPersonFlag;
use crate::vrm::humanoid_bone::insert_bone_holders;
use crate::vrm::instance::VrmSceneRoot;
use crate::vrm::spring_bone::{
    SpringCenterNode, SpringColliders, SpringJointProps, SpringJointState, SpringJoints, SpringRoot,
};
use crate::vrma::RetargetSource;

pub(crate) fn finalize(
    state: &mut VrmLoadState,
    load_context: &mut LoadContext<'_>,
    _scene: &gltf::Scene,
    world_root_id: Entity,
    scene_world: &mut World,
) {
    insert_rest_transforms(scene_world, world_root_id);
    build_spring_chains(state, scene_world);
    resolve_constraints(state, scene_world, world_root_id);
    build_expressions(state, scene_world, world_root_id);
    build_look_at(state, scene_world, world_root_id);
    name_and_animate(state, scene_world, world_root_id);
    insert_humanoid_bone_holders(scene_world, world_root_id);
    insert_source_path(load_context, scene_world, world_root_id);
    apply_first_person(state, load_context, scene_world);

    mark_initialized(scene_world, world_root_id);
}

/// Names the scene root after the model, then attaches the animation targets
/// and the player.
///
/// The order matters when `hips` is a top-level glTF node: its parent, and so
/// the root bone, is then the scene root itself, and the `Vrm::ROOT_BONE` name
/// that `ChildSearcher::find_root_bone` and the VRMA bake look the root bone up
/// by has to win over the model name, or no `.vrma` can ever play on it.
fn name_and_animate(
    state: &VrmLoadState,
    world: &mut World,
    root: Entity,
) {
    insert_name(state, world, root);
    setup_animation(state, world, root);
}

/// Bakes the crate-private [`VrmSceneRoot`] marker onto the root of the scene
/// asset.
///
/// The public [`Vrm`] + [`Initialized`](crate::vrm::Initialized) pair is
/// deliberately **not** written here. Moving root components onto the entity the
/// application spawned fires `On<Add, Vrm>` / `Added<Initialized>` on the target;
/// if the asset already carried them, those events would fire twice (once on the
/// child at instantiation, once on the user's entity after the move). Baking a
/// private marker and letting the attach observer
/// ([`attach_vrm_instance`](crate::vrm::instance)) insert the public pair
/// exactly once keeps the readiness lifecycle single-shot.
///
/// A raw `WorldAssetRoot(..)` spawn still ends up with `Vrm` + `Initialized` on
/// the child scene root: the attach observer puts them there because the owner
/// carries no [`VrmHandle`](crate::prelude::VrmHandle).
fn mark_initialized(
    world: &mut World,
    root: Entity,
) {
    world.entity_mut(root).insert(VrmSceneRoot);
}

/// Snapshots the rest pose of the whole scene.
///
/// Three components, because three consumers read three different frames:
///
/// * [`RestTransform`] / [`RestGlobalTransform`] on every entity with a
///   `Transform` — the local and world pose a bone returns to, used by node
///   constraints and gaze control;
/// * [`RestWorldTransform`] on the scene root — the model entity's own world
///   transform at snapshot time. The VRMA retarget strips this prefix from both
///   sides (`vrma::animation::bone_rotation::strip_world_prefix`) so a
///   retargeted pose does not depend on where the application placed the
///   avatar.
///
/// `GlobalTransform` is not propagated inside the loader's scratch world, so the
/// globals are composed by walking down from the root.
fn insert_rest_transforms(
    world: &mut World,
    root: Entity,
) {
    fn walk(
        world: &mut World,
        entity: Entity,
        parent_global: GlobalTransform,
        out: &mut Vec<(Entity, Transform, GlobalTransform)>,
    ) {
        let Some(local) = world.get::<Transform>(entity).copied() else {
            return;
        };
        let global = parent_global.mul_transform(local);
        out.push((entity, local, global));
        let children: Vec<Entity> = world
            .get::<Children>(entity)
            .map(|children| children.iter().collect())
            .unwrap_or_default();
        for child in children {
            walk(world, child, global, out);
        }
    }

    let mut poses = Vec::new();
    walk(world, root, GlobalTransform::IDENTITY, &mut poses);

    let root_global = poses
        .first()
        .map(|(_, _, global)| *global)
        .unwrap_or(GlobalTransform::IDENTITY);
    for (entity, local, global) in poses {
        world
            .entity_mut(entity)
            .insert((RestTransform(local), RestGlobalTransform(global)));
    }
    world
        .entity_mut(root)
        .insert(RestWorldTransform(root_global));
}

/// Builds the spring-bone chains from the crate's runtime components.
///
/// The runtime systems (`spring_bone::update`) simulate any chain that carries
/// `SpringRoot` + `SpringJointProps` + `SpringJointState`, so building all three
/// here leaves `VrmSpringBonePlugin` with nothing to set up.
fn build_spring_chains(
    state: &VrmLoadState,
    world: &mut World,
) {
    let Some(spring_bone) = state.spring_bone.as_ref() else {
        return;
    };

    // Resolve the colliders once: spec array index -> (entity, shape). A
    // collider node that was never spawned is dropped with a warning; the
    // remaining colliders of the group still apply.
    let mut resolved_colliders: Vec<Option<(Entity, ColliderShape)>> = Vec::new();
    for collider in &spring_bone.colliders {
        let resolved = match state.node_entities.get(&collider.node).copied() {
            Some(entity) => {
                // The shape is also a component of the collider node, as the
                // runtime initializer used to put it, so collider gizmos and app
                // code can find it there.
                world.entity_mut(entity).insert(collider.shape);
                Some((entity, collider.shape))
            }
            None => {
                vrm_warn!(format!(
                    "VRM springBone: collider refers to a missing node {}, skipping it",
                    collider.node
                ));
                None
            }
        };
        resolved_colliders.push(resolved);
    }

    for spring in &spring_bone.springs {
        // Resolve the joints; a chain that names a node nobody spawned cannot be
        // simulated, so it is skipped as a whole.
        let mut joints: Vec<Entity> = Vec::with_capacity(spring.joints.len());
        let mut props: Vec<SpringJointProps> = Vec::with_capacity(spring.joints.len());
        let mut broken = false;
        for joint in &spring.joints {
            let Some(&entity) = state.node_entities.get(&joint.node) else {
                vrm_warn!(format!(
                    "VRM springBone: joint refers to a missing node {}, skipping the chain",
                    joint.node
                ));
                broken = true;
                break;
            };
            joints.push(entity);
            // The schema materialises the specification defaults for every
            // omitted property (`dragForce` 0.5, `gravityDir` [0,-1,0],
            // `gravityPower` 0.0, `hitRadius` 0.0, `stiffness` 1.0), so a joint
            // only reads as `None` when the file wrote an explicit `null`.
            props.push(SpringJointProps {
                drag_force: joint.drag_force.unwrap_or(0.5),
                gravity_dir: joint.gravity_dir.map(Vec3::from).unwrap_or(Vec3::NEG_Y),
                gravity_power: joint.gravity_power.unwrap_or(0.0),
                hit_radius: joint.hit_radius.unwrap_or(0.0),
                stiffness: joint.stiffness.unwrap_or(1.0),
            });
        }
        if broken {
            continue;
        }
        if joints.len() < 2 {
            vrm_warn!(format!(
                "VRM springBone: spring `{}` has fewer than two joints; skipped",
                spring.name
            ));
            continue;
        }

        let center = spring
            .center
            .and_then(|node| state.node_entities.get(&node).copied());

        for pair in 0..joints.len() - 1 {
            let head = joints[pair];
            let tail = joints[pair + 1];
            let (Some(head_rest_global), Some(tail_rest_global), Some(head_rest_local)) = (
                world.get::<RestGlobalTransform>(head).copied(),
                world.get::<RestGlobalTransform>(tail).copied(),
                world.get::<RestTransform>(head).copied(),
            ) else {
                vrm_warn!(
                    "VRM springBone: a joint of the chain has no rest pose; the chain is skipped"
                );
                break;
            };
            // `reparented_to` instead of reading the tail's local translation:
            // a spring chain is allowed to skip nodes, so the next joint is not
            // necessarily a direct child of this one.
            let tail_in_head = tail_rest_global.0.reparented_to(&head_rest_global.0);
            world.entity_mut(head).insert((
                props[pair],
                SpringJointState::from_rest(
                    tail_in_head.translation.normalize_or_zero(),
                    tail_in_head.translation.length(),
                    head_rest_local.0,
                ),
            ));
        }
        // The last joint carries its properties but simulates no pair of its
        // own, because it is the tail of the last segment.
        if let (Some(&last), Some(&last_props)) = (joints.last(), props.last()) {
            world.entity_mut(last).insert(last_props);
        }

        let mut colliders: Vec<(Entity, ColliderShape)> = Vec::new();
        for group_index in spring.collider_groups.iter().flatten() {
            let Some(group) = spring_bone.collider_groups.get(*group_index) else {
                vrm_warn!(format!(
                    "VRM springBone: colliderGroups index {group_index} is out of range; \
                     the group is skipped"
                ));
                continue;
            };
            for collider_index in &group.colliders {
                if let Some(Some((entity, shape))) =
                    resolved_colliders.get(*collider_index as usize)
                {
                    colliders.push((*entity, *shape));
                }
            }
        }

        world.entity_mut(joints[0]).insert(SpringRoot {
            joints: SpringJoints(joints.clone()),
            colliders: SpringColliders(colliders),
            center_node: SpringCenterNode(center),
        });
    }
}

/// Turns the [`PendingNodeConstraint`]s into resolved [`VrmNodeConstraint`]s and
/// sorts the scene's constraint graph.
///
/// The pending component is **removed** again: it exists only because the source
/// node may not have been visited when the destination was, and it would
/// otherwise ship inside the scene asset with node indices that mean nothing in
/// an instantiated copy.
fn resolve_constraints(
    state: &VrmLoadState,
    world: &mut World,
    root: Entity,
) {
    let mut pendings: Vec<(Entity, PendingNodeConstraint)> = Vec::new();
    {
        let mut query = world.query::<(Entity, &PendingNodeConstraint)>();
        for (destination, pending) in query.iter(world) {
            pendings.push((destination, *pending));
        }
    }
    // Sorted by entity so the topological order below is reproducible: query
    // iteration order is not.
    pendings.sort_by_key(|(entity, _)| *entity);

    let mut constraints: Vec<(Entity, Entity)> = Vec::new();
    for (destination, pending) in pendings {
        let mut entity = world.entity_mut(destination);
        entity.remove::<PendingNodeConstraint>();
        let Some(&source) = state.node_entities.get(&pending.source_node) else {
            vrm_warn!(format!(
                "VRM node_constraint: the source node {} was not found; the constraint is \
                     skipped",
                pending.source_node
            ));
            continue;
        };
        entity.insert(VrmNodeConstraint {
            source,
            weight: pending.weight,
            kind: pending.kind,
        });
        constraints.push((destination, source));
    }

    world
        .entity_mut(root)
        .insert(ConstraintExecutionOrder(toposort_constraints(&constraints)));
}

/// Kahn's algorithm over the constraints of one scene.
///
/// A node is a graph vertex when it is the *destination* of some constraint;
/// there is an edge `source -> destination` when the source is itself a
/// destination, i.e. when evaluating the destination requires the source's
/// rotation to be final.
///
/// The specification forbids cycles. One is reported and the remaining
/// constraints are appended unordered rather than dropped, so a malformed
/// exporter costs ordering, not behaviour.
fn toposort_constraints(constraints: &[(Entity, Entity)]) -> Vec<Entity> {
    let destination_index: HashMap<Entity, usize> = constraints
        .iter()
        .enumerate()
        .map(|(index, (destination, _))| (*destination, index))
        .collect();
    let mut indegree = vec![0usize; constraints.len()];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); constraints.len()];
    for (index, (_, source)) in constraints.iter().enumerate() {
        if let Some(&source_index) = destination_index.get(source) {
            outgoing[source_index].push(index);
            indegree[index] += 1;
        }
    }

    let mut ready: Vec<usize> = (0..constraints.len())
        .filter(|index| indegree[*index] == 0)
        .collect();
    let mut order = Vec::with_capacity(constraints.len());
    while let Some(index) = ready.pop() {
        order.push(constraints[index].0);
        for &next in &outgoing[index] {
            let Some(degree) = indegree.get_mut(next) else {
                continue;
            };
            *degree -= 1;
            if *degree == 0 {
                ready.push(next);
            }
        }
    }

    if order.len() < constraints.len() {
        vrm_warn!(
            "VRM node_constraint: the constraints of this scene contain a dependency cycle; \
             the remaining constraints are evaluated unordered"
        );
        for (index, (destination, _)) in constraints.iter().enumerate() {
            if indegree[index] > 0 {
                order.push(*destination);
            }
        }
    }
    order
}

/// Builds the expression components on the scene root.
///
/// Preset **and** custom expressions are both included: `Expressions::custom`
/// is a first-class part of `VRMC_vrm.expressions` and upstream's runtime path
/// silently dropped it, which lost every custom expression of an avatar.
fn build_expressions(
    state: &VrmLoadState,
    world: &mut World,
    root: Entity,
) {
    let Some(vrm) = &state.vrm else {
        return;
    };
    let Some(expressions) = &vrm.expressions else {
        return;
    };

    let mut weights = VrmExpressionWeights::default();
    let mut finals = EffectiveExpressionWeights::default();
    let mut binds = ExpressionMorphBinds::default();
    let mut settings = ExpressionSettings::default();

    let preset = expressions
        .preset
        .iter()
        .map(|(name, expression)| (name, expression, true));
    let custom = expressions
        .custom
        .iter()
        .map(|(name, expression)| (name, expression, false));

    for (name, expression, is_preset) in preset.chain(custom) {
        let mut bind_list = Vec::new();
        for bind in expression.morph_target_binds.iter().flatten() {
            let Some(&target) = state.node_entities.get(&bind.node) else {
                vrm_warn!(format!(
                    "VRM expressions: a morph bind of `{name}` refers to the missing node {}",
                    bind.node
                ));
                continue;
            };
            // The bind drives a morph index of a mesh, and only a node that
            // carries `MorphWeights` has any. `bevy_gltf` puts
            // `MorphWeights` on the node and `MeshMorphWeights` on each of its
            // primitives (`loader/mod.rs`), so this is
            // also the check that the node has a mesh at all.
            if !world.get::<MorphWeights>(target).is_some() {
                vrm_warn!(format!(
                    "VRM expressions: node {} of the morph binds of `{name}` carries no morph \
                     weights; the bind is skipped",
                    bind.node
                ));
                continue;
            }
            bind_list.push(MorphBind {
                target,
                index: bind.index,
                weight: bind.weight,
            });
        }
        if !expression.material_color_binds.is_empty()
            || !expression.texture_transform_binds.is_empty()
        {
            // Both bind kinds are parsed by the schema and kept, so a follow-up
            // can apply them without another migration; nothing applies them
            // today, and pretending otherwise would be worse than saying so.
            vrm_warn!(format!(
                "VRM expressions: the material color and texture transform binds of `{name}` \
                 are parsed but not applied"
            ));
        }

        weights.0.insert(name.clone(), 0.0);
        finals.0.insert(name.clone(), 0.0);
        binds.0.insert(name.clone(), bind_list);
        settings.0.insert(
            name.clone(),
            ExpressionSetting {
                is_binary: expression.is_binary,
                category: if is_preset {
                    ExpressionCategory::from_preset_name(name)
                } else {
                    ExpressionCategory::Other
                },
                override_blink: ExpressionOverrideType::parse(&expression.override_blink),
                override_look_at: ExpressionOverrideType::parse(&expression.override_look_at),
                override_mouth: ExpressionOverrideType::parse(&expression.override_mouth),
            },
        );
    }

    world
        .entity_mut(root)
        .insert((weights, finals, binds, settings));
}

/// Puts the parsed `VRMC_vrm.lookAt` on the scene root.
///
/// The component is the crate's existing [`LookAtProperties`], with the
/// specification defaults already applied by its `Deserialize`
/// (`RangeMap` defaults to `inputMaxValue` 90.0 / `outputScale` 10.0, `type` to
/// `bone`), so there is exactly one look-at component in the crate.
fn build_look_at(
    state: &VrmLoadState,
    world: &mut World,
    root: Entity,
) {
    let Some(vrm) = &state.vrm else {
        return;
    };
    let Some(look_at) = vrm.look_at.clone() else {
        return;
    };

    world.entity_mut(root).insert(look_at);
}

/// Attaches the animation targets and the player.
///
/// # Bone-name ids, not glTF path ids
///
/// Every humanoid bone gets
/// [`AnimationTargetId::from_name`] of its **VRM bone name**, which is the id a
/// baked `.vrma` clip addresses, so one clip plays on any avatar. That is a
/// different id from the path-based one `bevy_gltf` computes for a file's own
/// animations (`loader/mod.rs`); a `.vrm` declares no glTF animation in
/// practice, so no path id exists to clobber, and the two systems are separated
/// by `AnimatedBy` anyway — `bevy_gltf`'s targets point at the animation root it
/// picked, ours point at [`Vrm::ROOT_BONE`].
///
/// # Why the player sits on the root bone, not on the scene root
///
/// bevy resolves a target's player through the entity named by its
/// [`AnimatedBy`] component, and that entity must carry both an
/// `AnimationPlayer` and an `AnimationGraphHandle`
/// (`bevy_animation/src/lib.rs`). The VRMA side writes the graph
/// handle onto the **root bone** (`insert_animation_graph_into_root_bone` in
/// `src/vrma/animation/animation_graph.rs`),
/// [`PlayVrma`](crate::prelude::PlayVrma) plays on the **root bone**
/// (`src/vrma/animation/play.rs`), and
/// [`VrmAnimation::all_finished`](crate::prelude::VrmAnimation::all_finished)
/// reads the **root bone**'s player. So the root bone is the animation root and
/// every target has to name it. A player left on the scene root would never be
/// the player of any target, i.e. inert.
///
/// The scene root still carries the synthetic target
/// [`vrm_root_animation_target`], so curves that address the avatar as a whole
/// (expression weights) have something to bind to, and it is a target of the same
/// player.
fn setup_animation(
    state: &VrmLoadState,
    world: &mut World,
    root: Entity,
) {
    // The root bone is the parent of `hips`:
    // `ChildSearcher::find_root_bone` and the VRMA retarget both look it up by
    // `Vrm::ROOT_BONE`.
    let root_bone = state
        .bone_nodes
        .get("hips")
        .and_then(|node| state.node_entities.get(node))
        .copied()
        .and_then(|hips| world.get::<ChildOf>(hips).map(ChildOf::parent));
    // A file without a `hips` bone has no root bone and therefore no humanoid
    // rig at all. Fall back to the scene root so a user-supplied graph still has
    // a player; the VRMA path cannot attach to a rig that does not exist, and
    // it bails on `find_root_bone` first anyway.
    let animation_root = root_bone.unwrap_or(root);

    for (bone, node) in &state.bone_nodes {
        let Some(&entity) = state.node_entities.get(node) else {
            continue;
        };
        world.entity_mut(entity).insert((
            bone_target_id(bone),
            AnimatedBy(animation_root),
            RetargetSource,
        ));
    }

    world
        .entity_mut(root)
        .insert((vrm_root_animation_target(), AnimatedBy(animation_root)));

    if let Some(root_bone) = root_bone {
        world.entity_mut(root_bone).insert((
            Name::new(Vrm::ROOT_BONE),
            AnimationPlayer::default(),
            AnimationTransitions::default(),
            RetargetSource,
        ));
    } else {
        world.entity_mut(root).insert(AnimationPlayer::default());
    }
}

/// Puts the `<Bone>BoneEntity` holders and the bone markers on the scene.
///
/// The holders are what the runtime systems that address a bone *through* the
/// avatar read, and a scene that has none is inert:
///
/// * [`track_looking_target`](crate::vrm::look_at::track_looking_target) needs
///   [`HeadBoneEntity`](crate::prelude::HeadBoneEntity) plus both eye holders,
///   so `LookAt` does nothing without them
///   (`src/vrm/look_at.rs`);
/// * [`track_body_tracking`](crate::vrm::body_tracking) needs
///   [`HeadBoneEntity`](crate::prelude::HeadBoneEntity) and optionally the
///   neck / chest / spine holders (`src/vrm/body_tracking.rs`);
///
/// The first-person auto split is a separate load-time step: it resolves the
/// head subtree from the document's node indices rather than these holders.
///
/// The rest poses from [`insert_rest_transforms`] are already on the bones;
/// [`Vrm`] and [`Initialized`](crate::vrm::Initialized) are added by the
/// scene-ready binding after the holders reach the runtime root.
///
/// # Why the mapping is read off `VrmBone`, not off `bone_nodes`
///
/// [`VrmNodeIndex`](crate::prelude::VrmNodeIndex) and [`VrmBone`] are written per
/// node in [`nodes::process_node`](super::nodes::process_node), which also
/// resolves a node claimed by two humanoid bones to a single name. The world is
/// therefore the authority on *which entity is which bone*, and the query below
/// is scoped to the scene's own world, which `bone_nodes` — accumulated across
/// every scene of the file — is not.
///
/// # The holders follow the avatar
///
/// The entity ids minted here belong to the loader's scratch world. The scene
/// spawn pipeline remaps them through `Component::map_entities`, which
/// `#[derive(Component)]` generates from `#[entities]` — and
/// `entity_component!` (`src/macros.rs`) does write that attribute on the
/// holder's field, as it does on
/// [`VrmNodeConstraint::source`](crate::prelude::VrmNodeConstraint::source)
/// (`src/vrm/components.rs`). An instantiated avatar therefore holds its
/// own bones, and gaze control sees a live entity rather than a stale id. See
/// [`crate::vrm::humanoid_bone::insert_bone_holders`].
fn insert_humanoid_bone_holders(
    world: &mut World,
    root: Entity,
) {
    let mut bones: Vec<(VrmBone, Entity)> = {
        let mut query = world.query::<(Entity, &VrmBone)>();
        query
            .iter(world)
            .map(|(entity, bone)| (bone.clone(), entity))
            .collect()
    };
    // Query iteration order is undefined; the inserts do not depend on it, but
    // sorting keeps the result reproducible for a log or an assertion.
    bones.sort_by_key(|(_, entity)| *entity);
    if bones.is_empty() {
        return;
    }

    {
        // `insert_bone_holders` writes through `Commands`, which `bevy_gltf`'s
        // loader does not have. The world's own command queue takes that role —
        // `bevy_ecs` documents these two calls for exactly this case.
        let mut commands = world.commands();
        insert_bone_holders(root, &mut commands, &bones);
    }
    // Applied before `WorldAsset::new` freezes the world, so the holders are part
    // of the asset the scene spawn pipeline copies out of.
    world.flush();
}

/// Records the file the scene was loaded from as [`VrmPath`].
///
/// [`LoadContext::path`] is observable here, and it is the *file*: `bevy_gltf`
/// derives the scene's context from the file's with
/// `begin_labeled_asset` (`loader/mod.rs`), which clones the path as it is
/// and adds the `Scene0` label only when the asset is registered. So this is the path the user passed to
/// `AssetServer::load`, and
/// [`RequestDetachVrm`](crate::vrm::detach::RequestDetachVrm) — which removes
/// `VrmPath` again — has it to remove.
fn insert_source_path(
    load_context: &LoadContext<'_>,
    world: &mut World,
    root: Entity,
) {
    world
        .entity_mut(root)
        .insert(VrmPath::new(load_context.path().path()));
}

/// Names the scene root after `VRMC_vrm.meta.name`, falling back to `VRM`, in
/// place of the `Scene0` that `bevy_gltf` gave it.
fn insert_name(
    state: &VrmLoadState,
    world: &mut World,
    root: Entity,
) {
    let name = state
        .vrm
        .as_ref()
        .and_then(|vrm| vrm.meta.as_ref()?.name.clone())
        .unwrap_or_else(|| "VRM".to_owned());
    world.entity_mut(root).insert(Name::new(name));
}

/// Records the `VRMC_vrm.firstPerson` decisions on the scene.
///
/// Three passes, in this order:
///
/// 1. the per-primitive `auto` classifications recorded while the meshes were
///    built;
/// 2. the explicit node annotations, which *override* the `auto` result — an
///    `auto` annotation is skipped here, because it has already been honoured per
///    primitive;
/// 3. the head copies, spawned as siblings of the body half.
///
/// Every mesh stays on the default render layer: first person is opt-in, and
/// [`RequestEnableFirstPerson`](crate::prelude::RequestEnableFirstPerson) turns
/// the recorded [`FirstPersonView`]s into layers.
fn apply_first_person(
    state: &VrmLoadState,
    load_context: &mut LoadContext<'_>,
    world: &mut World,
) {
    record_first_person_views(state, world);
    let head_meshes: HashMap<(usize, usize), Handle<Mesh>> = state
        .first_person
        .head_mesh_labels
        .iter()
        .map(|(key, label)| (*key, load_context.get_label_handle(label.clone())))
        .collect();
    spawn_head_copies(state, &head_meshes, world);
}

/// Passes 1 and 2 of [`apply_first_person`].
fn record_first_person_views(
    state: &VrmLoadState,
    world: &mut World,
) {
    for (key, class) in &state.first_person.classes {
        if *class != AutoClass::ThirdPersonOnly {
            continue;
        }
        for &entity in state.primitive_entities.get(key).into_iter().flatten() {
            world
                .entity_mut(entity)
                .insert(FirstPersonView::ThirdPersonOnly);
        }
    }

    let Some(first_person) = state.vrm.as_ref().and_then(|vrm| vrm.first_person.as_ref()) else {
        return;
    };
    let annotated: HashSet<Entity> = first_person
        .mesh_annotations
        .iter()
        .filter_map(|annotation| state.node_entities.get(&annotation.node).copied())
        .collect();
    for annotation in &first_person.mesh_annotations {
        let view = match annotation.first_person_flag {
            FirstPersonFlag::FirstPersonOnly => Some(FirstPersonView::FirstPersonOnly),
            FirstPersonFlag::ThirdPersonOnly => Some(FirstPersonView::ThirdPersonOnly),
            FirstPersonFlag::Both => None,
            // Handled per primitive in pass 1 and 3.
            FirstPersonFlag::Auto => continue,
        };
        let Some(&entity) = state.node_entities.get(&annotation.node) else {
            vrm_warn!(format!(
                "VRM firstPerson: meshAnnotations names the missing node {}",
                annotation.node
            ));
            continue;
        };
        record_view_recursive(world, entity, view, &annotated);
    }
}

/// Spawns pass 3 of [`apply_first_person`].
///
/// The copy inherits everything that makes the body half render: the transform,
/// the skin, the material, the morph-weight reference and the bounds. Only the
/// recorded view differs.
fn spawn_head_copies(
    state: &VrmLoadState,
    head_meshes: &HashMap<(usize, usize), Handle<Mesh>>,
    world: &mut World,
) {
    // One copy per node that instances the split primitive.
    let copies = head_meshes.iter().flat_map(|(key, head_mesh)| {
        let sources = state.primitive_entities.get(key);
        if sources.is_none() {
            vrm_warn!(format!(
                "VRM firstPerson: no entity was recorded for mesh {} primitive {}, so its head \
                 part is not spawned",
                key.0, key.1
            ));
        }
        sources
            .into_iter()
            .flatten()
            .map(move |&source| (key, head_mesh, source))
    });
    for (key, head_mesh, source) in copies {
        let Some(parent) = world.get::<ChildOf>(source).map(ChildOf::parent) else {
            vrm_warn!(format!(
                "VRM firstPerson: the body half of mesh {} primitive {} has no parent",
                key.0, key.1
            ));
            continue;
        };
        let name = world
            .get::<Name>(source)
            .map(|name| format!("{name}/VrmHeadOnly"))
            .unwrap_or_else(|| "VrmHeadOnly".to_owned());
        let transform = world.get::<Transform>(source).copied().unwrap_or_default();
        let visibility = world.get::<Visibility>(source).copied().unwrap_or_default();

        // Everything the body half needs in order to render at all. Read before
        // spawning: `World::spawn` mutably borrows the world for as long as the
        // returned `EntityWorldMut` lives.
        //
        // `MeshMorphWeights` is what drives the node's `MorphWeights`, so the
        // head half has to reference the same node or its expressions do nothing.
        let morph_weights = world.get::<MeshMorphWeights>(source).cloned();
        let skinned = world.get::<SkinnedMesh>(source).cloned();
        // The source's `Aabb` is a superset of the head half's, which only costs
        // a little frustum-culling precision; the skinned-mesh bounds markers
        // come along because they are what makes those bounds track the animated
        // pose at all.
        let aabb = world.get::<Aabb>(source).copied();
        let dynamic_bounds = world.get::<DynamicSkinnedMeshBounds>(source).is_some();
        let no_frustum_culling = world.get::<NoFrustumCulling>(source).is_some();
        let material = world.get::<MeshMaterial3d<MToonMaterial>>(source).cloned();
        let fallback_material = world
            .get::<MeshMaterial3d<StandardMaterial>>(source)
            .cloned();

        let mut head = world.spawn((
            Name::new(name),
            Mesh3d(head_mesh.clone()),
            transform,
            visibility,
            ChildOf(parent),
            VrmHeadOnly,
            FirstPersonView::ThirdPersonOnly,
        ));
        if let Some(morph_weights) = morph_weights {
            head.insert(morph_weights);
        }
        if let Some(skinned) = skinned {
            head.insert(skinned);
        }
        if let Some(aabb) = aabb {
            head.insert(aabb);
        }
        if let Some(material) = material {
            head.insert(material);
        } else if let Some(material) = fallback_material {
            head.insert(material);
        }
        if dynamic_bounds {
            head.insert(DynamicSkinnedMeshBounds);
        }
        if no_frustum_culling {
            head.insert(NoFrustumCulling);
        }
    }
}

/// Records `view` on every `Mesh3d` entity of the annotated subtree, down to
/// the next annotated node: the nearest annotation wins, as in
/// [`first_person`](super::first_person).
fn record_view_recursive(
    world: &mut World,
    entity: Entity,
    view: Option<FirstPersonView>,
    annotated: &HashSet<Entity>,
) {
    if world.get::<Mesh3d>(entity).is_some() {
        let mut mesh = world.entity_mut(entity);
        match view {
            Some(view) => {
                mesh.insert(view);
            }
            None => {
                mesh.remove::<FirstPersonView>();
            }
        }
    }
    let children: Vec<Entity> = world
        .get::<Children>(entity)
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    for child in children {
        if !annotated.contains(&child) {
            record_view_recursive(world, child, view, annotated);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::{
        BodyTracking, ChestBoneEntity, Head, HeadBoneEntity, Hips, HipsBoneEntity, Initialized,
        LeftEye, LeftEyeBoneEntity, LookAt, NeckBoneEntity, RightEyeBoneEntity, SpineBoneEntity,
        VrmConstraintKind,
    };
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::body_tracking::BodyTrackingPlugin;
    use crate::vrm::gltf::extensions::vrmc_vrm::VrmcVrm;
    use crate::vrm::look_at::LookAtPlugin;
    use bevy::animation::AnimationTargetId;
    use bevy::camera::visibility::RenderLayers;

    /// `VrmcVrm` parsed from a JSON literal, so the tests below exercise the
    /// same schema the loader reads.
    fn vrm(json: &str) -> std::sync::Arc<VrmcVrm> {
        std::sync::Arc::new(serde_json::from_str(json).expect("the fixture should parse"))
    }

    fn state_with(vrm: std::sync::Arc<VrmcVrm>) -> VrmLoadState {
        VrmLoadState {
            vrm: Some(vrm),
            ..Default::default()
        }
    }

    fn node(index: usize) -> Entity {
        Entity::from_raw_u32(index as u32).expect("a valid entity index")
    }

    /// A scene: `root -> child -> grandchild`, each with a `Transform`.
    fn scene_world() -> (World, Entity, Entity, Entity) {
        let mut world = World::new();
        let root = world.spawn(Transform::from_xyz(1.0, 0.0, 0.0)).id();
        let child = world
            .spawn((Transform::from_xyz(0.0, 2.0, 0.0), ChildOf(root)))
            .id();
        let grandchild = world
            .spawn((Transform::from_xyz(0.0, 0.0, 3.0), ChildOf(child)))
            .id();
        (world, root, child, grandchild)
    }

    /// The VRMA retarget strips `RestWorldTransform` from both sides, so all
    /// three rest components have to exist and agree with each other.
    #[test]
    fn rest_transforms_cover_the_whole_scene() -> TestResult {
        let (mut world, root, child, grandchild) = scene_world();

        insert_rest_transforms(&mut world, root);

        assert_eq!(
            world.get::<RestTransform>(root).map(|rest| rest.0),
            Some(Transform::from_xyz(1.0, 0.0, 0.0))
        );
        assert_eq!(
            world.get::<RestGlobalTransform>(child).map(|rest| rest.0),
            Some(GlobalTransform::from(Transform::from_xyz(1.0, 2.0, 0.0)))
        );
        assert_eq!(
            world
                .get::<RestGlobalTransform>(grandchild)
                .map(|rest| rest.0),
            Some(GlobalTransform::from(Transform::from_xyz(1.0, 2.0, 3.0)))
        );
        // The model's own world transform, so a retarget does not depend on
        // where the application placed the avatar.
        assert_eq!(
            world.get::<RestWorldTransform>(root).map(|rest| rest.0),
            world.get::<RestGlobalTransform>(root).map(|rest| rest.0)
        );
        success!()
    }

    /// `bevy_gltf` builds every scene of a file in a fresh `World`, so an entity
    /// a scene resolved a node to means nothing in the next scene.
    #[test]
    fn a_scene_does_not_resolve_nodes_through_the_previous_scene() -> TestResult {
        let mut state = VrmLoadState {
            spring_bone: Some(std::sync::Arc::new(serde_json::from_str(
                r#"{"specVersion": "1.0", "springs": [{"joints": [{"node": 0}, {"node": 1}]}]}"#,
            )?)),
            ..Default::default()
        };
        // Scene 0 instantiates the chain.
        let mut first = World::new();
        let head = first.spawn(Transform::default()).id();
        let tail = first.spawn((Transform::default(), ChildOf(head))).id();
        state.node_entities = [(0, head), (1, tail)].into_iter().collect();
        state.primitive_entities.insert((0, 0), vec![tail]);
        state.end_scene();

        // Scene 1 instantiates neither node, but its own entities get the same
        // ids.
        let mut second = World::new();
        let unrelated = second.spawn(Transform::default()).id();
        second.spawn((Transform::default(), ChildOf(unrelated)));
        assert_eq!(unrelated, head);
        insert_rest_transforms(&mut second, unrelated);
        build_spring_chains(&state, &mut second);

        assert!(second.get::<SpringRoot>(unrelated).is_none());
        assert!(state.primitive_entities.is_empty());
        success!()
    }

    /// A collider's shape is a component of its node, besides being listed in
    /// the `SpringColliders` of the chains that use it.
    #[test]
    fn a_collider_node_carries_its_shape() -> TestResult {
        let state = VrmLoadState {
            spring_bone: Some(std::sync::Arc::new(serde_json::from_str(
                r#"{
                    "specVersion": "1.0",
                    "colliders": [{"node": 2, "shape": {"sphere": {"offset": [0, 0, 0], "radius": 0.25}}}],
                    "colliderGroups": [{"colliders": [0]}],
                    "springs": [{"joints": [{"node": 0}, {"node": 1}], "colliderGroups": [0]}]
                }"#,
            )?)),
            ..Default::default()
        };
        let mut world = World::new();
        let head = world.spawn(Transform::default()).id();
        let tail = world
            .spawn((Transform::from_xyz(0.0, 1.0, 0.0), ChildOf(head)))
            .id();
        let collider = world.spawn(Transform::default()).id();
        let state = VrmLoadState {
            node_entities: [(0, head), (1, tail), (2, collider)].into_iter().collect(),
            ..state
        };
        insert_rest_transforms(&mut world, head);

        build_spring_chains(&state, &mut world);

        let shape = world
            .get::<ColliderShape>(collider)
            .copied()
            .expect("the collider node carries its shape");
        assert!(matches!(shape, ColliderShape::Sphere(sphere) if sphere.radius == 0.25));
        assert_eq!(
            world
                .get::<SpringRoot>(head)
                .map(|root| root.colliders.0.clone()),
            Some(vec![(collider, shape)])
        );
        success!()
    }

    /// Sources are evaluated before the destinations that read them.
    #[test]
    fn constraints_are_sorted_source_before_destination() -> TestResult {
        // `(destination, source)`: `a` reads `b`, `b` reads `c`, `c` reads `d`,
        // and `d` is not a destination itself, so the graph is a chain.
        let (a, b, c, d) = (node(1), node(2), node(3), node(4));
        let order = toposort_constraints(&[(a, b), (b, c), (c, d)]);

        let position = |entity: Entity| {
            order
                .iter()
                .position(|candidate| *candidate == entity)
                .unwrap_or_else(|| panic!("every constraint is listed"))
        };
        assert!(position(b) < position(a), "a reads b");
        assert!(position(c) < position(b), "b reads c");
        assert!(
            !order.contains(&d),
            "only destinations are listed; a source that nothing constrains is not"
        );
        success!()
    }

    /// The specification forbids cycles. One is reported, not dropped: the
    /// remaining constraints are appended unordered.
    #[test]
    fn a_constraint_cycle_still_lists_every_destination() -> TestResult {
        let (a, b) = (node(1), node(2));
        let order = toposort_constraints(&[(a, b), (b, a)]);

        assert_eq!(order.len(), 2, "neither constraint is dropped");
        success!()
    }

    /// The loader-only marker must not survive into the scene asset, and the
    /// resolved constraint must point at an entity of the same world.
    #[test]
    fn pending_constraints_are_resolved_and_removed() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let source = world.spawn_empty().id();
        let destination = world.spawn_empty().id();
        world.entity_mut(destination).insert(PendingNodeConstraint {
            source_node: 7,
            weight: 0.5,
            kind: VrmConstraintKind::Aim {
                aim_axis: Dir3::NEG_Z,
            },
        });
        let state = VrmLoadState {
            node_entities: [(7, source)].into_iter().collect(),
            ..Default::default()
        };

        resolve_constraints(&state, &mut world, root);

        assert!(
            world.get::<PendingNodeConstraint>(destination).is_none(),
            "the pending marker would ship inside the scene asset"
        );
        let constraint = world
            .get::<VrmNodeConstraint>(destination)
            .expect("the constraint is resolved");
        assert_eq!(constraint.source, source);
        assert_eq!(constraint.weight, 0.5);
        assert_eq!(
            world
                .get::<ConstraintExecutionOrder>(root)
                .map(|order| order.destinations()),
            Some([destination].as_slice())
        );

        // A constraint whose source node was never spawned is dropped, and the
        // pending marker still goes away.
        let orphan = world.spawn_empty().id();
        world.entity_mut(orphan).insert(PendingNodeConstraint {
            source_node: 999,
            weight: 1.0,
            kind: VrmConstraintKind::Rotation,
        });
        resolve_constraints(&state, &mut world, root);
        assert!(world.get::<PendingNodeConstraint>(orphan).is_none());
        assert!(world.get::<VrmNodeConstraint>(orphan).is_none());

        success!()
    }

    /// Preset *and* custom expressions reach the root, and a bind is only kept
    /// when its node can actually carry `MorphWeights`.
    #[test]
    fn expressions_include_custom_and_require_morph_weights() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let preset_target = world.spawn(MorphWeights::new(vec![0.0; 3], None)?).id();
        let custom_target = world.spawn(MorphWeights::new(vec![0.0; 2], None)?).id();
        // A node without `MorphWeights`: its bind must be skipped.
        let weightless = world.spawn_empty().id();
        let state = state_with(vrm(r#"{
                "specVersion": "1.0",
                "humanoid": {"humanBones": {}},
                "expressions": {
                    "preset": {"blink": {"isBinary": true, "morphTargetBinds": [
                        {"node": 1, "index": 0, "weight": 1.0},
                        {"node": 3, "index": 0, "weight": 1.0}
                    ]}},
                    "custom": {"Wave": {"morphTargetBinds": [
                        {"node": 2, "index": 1, "weight": 0.5}
                    ]}}
                }
            }"#));
        let state = VrmLoadState {
            node_entities: [(1, preset_target), (2, custom_target), (3, weightless)]
                .into_iter()
                .collect(),
            ..state
        };

        build_expressions(&state, &mut world, root);

        let weights = world
            .get::<VrmExpressionWeights>(root)
            .expect("the root carries the expression weights");
        assert_eq!(weights.0.get("blink"), Some(&0.0));
        assert_eq!(
            weights.0.get("Wave"),
            Some(&0.0),
            "`Expressions::custom` is part of the specification and is not dropped"
        );

        let binds = world
            .get::<ExpressionMorphBinds>(root)
            .expect("the root carries the morph binds");
        assert_eq!(binds.0["blink"].len(), 1, "node 3 has no `MorphWeights`");
        assert_eq!(binds.0["blink"][0].target, preset_target);
        assert_eq!(binds.0["Wave"][0].target, custom_target);
        assert_eq!(binds.0["Wave"][0].weight, 0.5);

        let settings = world
            .get::<ExpressionSettings>(root)
            .expect("the root carries the per-expression settings");
        assert!(settings.0["blink"].is_binary);
        assert_eq!(
            settings.0["blink"].category,
            ExpressionCategory::Blink,
            "a preset name is classified"
        );
        assert_eq!(
            settings.0["Wave"].category,
            ExpressionCategory::Other,
            "a custom expression is not a preset category"
        );

        success!()
    }

    /// `overrideBlink` / `overrideLookAt` / `overrideMouth` are spec strings and
    /// are parsed by the schema already.
    #[test]
    fn expression_overrides_are_typed() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let state = state_with(vrm(r#"{
                "specVersion": "1.0",
                "humanoid": {"humanBones": {}},
                "expressions": {"preset": {"happy": {"overrideMouth": "block"}}}
            }"#));

        build_expressions(&state, &mut world, root);

        let settings = world.get::<ExpressionSettings>(root).unwrap();
        assert_eq!(
            settings.0["happy"].override_mouth,
            ExpressionOverrideType::Block
        );
        assert_eq!(
            settings.0["happy"].override_blink,
            ExpressionOverrideType::None
        );
        success!()
    }

    /// The scene root is an animation target under the synthetic name, and the
    /// `hips` bone under its VRM bone name.
    #[test]
    fn animation_targets_are_attached_to_the_root_and_the_bones() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        // Distinct from the scene root, because the two carry different roles:
        // the root bone is the animation root.
        let root_bone = world.spawn(ChildOf(root)).id();
        let hips = world.spawn(ChildOf(root_bone)).id();
        let state = VrmLoadState {
            bone_nodes: [("hips".to_owned(), 0)].into_iter().collect(),
            node_entities: [(0, hips)].into_iter().collect(),
            ..Default::default()
        };

        setup_animation(&state, &mut world, root);

        assert_eq!(
            world.get::<AnimationTargetId>(root).copied(),
            Some(vrm_root_animation_target()),
            "root-level curves address the avatar through a synthetic target"
        );
        assert_eq!(
            world.get::<AnimationTargetId>(hips).copied(),
            Some(bone_target_id("hips")),
            "a bone is addressed by its VRM bone name, so one clip retargets to any avatar"
        );
        // Both the root and the bone are targets of the player on the root bone,
        // which is where the VRMA graph handle is written.
        assert_eq!(
            world.get::<AnimatedBy>(hips).map(|animated| animated.0),
            Some(root_bone)
        );
        assert_eq!(
            world.get::<AnimatedBy>(root).map(|animated| animated.0),
            Some(root_bone)
        );
        // `ChildSearcher::find_root_bone` and the VRMA retarget look the root
        // bone up by this name, and `PlayVrma` plays on the player it holds.
        assert_eq!(
            world.get::<Name>(root_bone).map(|name| name.as_str()),
            Some(Vrm::ROOT_BONE)
        );
        assert!(world.get::<AnimationPlayer>(root_bone).is_some());
        assert!(world.get::<AnimationTransitions>(root_bone).is_some());
        assert!(world.get::<RetargetSource>(root_bone).is_some());
        // The scene root is not the animation root, so it must not hold a
        // player of its own: no `AnimatedBy` points at it.
        assert!(world.get::<AnimationPlayer>(root).is_none());
        success!()
    }

    /// A `hips` listed directly in the glTF scene is a child of the scene root,
    /// so the scene root is the root bone. It has to keep `Vrm::ROOT_BONE`, the
    /// name the runtime finds the root bone by, rather than the model name.
    #[test]
    fn a_top_level_hips_keeps_the_root_bone_name_on_the_scene_root() -> TestResult {
        let mut world = World::new();
        let root = world.spawn(Name::new("Scene0")).id();
        let hips = world.spawn(ChildOf(root)).id();
        let state = VrmLoadState {
            bone_nodes: [("hips".to_owned(), 0)].into_iter().collect(),
            node_entities: [(0, hips)].into_iter().collect(),
            ..state_with(vrm(
                r#"{"specVersion": "1.0", "humanoid": {"humanBones": {}}, "meta": {"name": "Alicia"}}"#,
            ))
        };

        name_and_animate(&state, &mut world, root);

        assert_eq!(
            world.get::<Name>(root).map(|name| name.as_str()),
            Some(Vrm::ROOT_BONE)
        );
        assert!(world.get::<AnimationPlayer>(root).is_some());
        assert_eq!(
            world.get::<AnimatedBy>(hips).map(|animated| animated.0),
            Some(root)
        );
        success!()
    }

    /// The baked marker is the crate-private [`VrmSceneRoot`], not the public
    /// `Vrm` + `Initialized` pair: those are inserted once by the attach observer,
    /// so a move onto the application's entity cannot fire their lifecycle events
    /// a second time.
    #[test]
    fn finalize_bakes_the_private_scene_root_marker() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();

        mark_initialized(&mut world, root);

        assert!(world.get::<VrmSceneRoot>(root).is_some());
        assert!(world.get::<Vrm>(root).is_none());
        assert!(world.get::<Initialized>(root).is_none());
        success!()
    }

    #[test]
    fn the_scene_root_is_named_after_the_meta_name() -> TestResult {
        let mut world = World::new();
        let named = world.spawn(Name::new("Scene0")).id();
        let unnamed = world.spawn(Name::new("Scene0")).id();

        insert_name(
            &state_with(vrm(r#"{
                "specVersion": "1.0",
                "humanoid": {"humanBones": {}},
                "meta": {"name": "Alicia Solid"}
            }"#)),
            &mut world,
            named,
        );
        insert_name(
            &state_with(vrm(
                r#"{"specVersion": "1.0", "humanoid": {"humanBones": {}}}"#,
            )),
            &mut world,
            unnamed,
        );

        assert_eq!(
            world.get::<Name>(named).map(Name::as_str),
            Some("Alicia Solid")
        );
        assert_eq!(world.get::<Name>(unnamed).map(Name::as_str), Some("VRM"));
        success!()
    }

    /// A file that declares no `hips` bone has no root bone. The scene root then
    /// keeps the player, so a user-supplied graph still has somewhere to live.
    #[test]
    fn a_scene_without_hips_keeps_a_player_on_the_scene_root() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let spine = world.spawn(ChildOf(root)).id();
        let state = VrmLoadState {
            bone_nodes: [("spine".to_owned(), 0)].into_iter().collect(),
            node_entities: [(0, spine)].into_iter().collect(),
            ..Default::default()
        };

        setup_animation(&state, &mut world, root);

        assert!(world.get::<AnimationPlayer>(root).is_some());
        assert_eq!(
            world.get::<AnimatedBy>(spine).map(|animated| animated.0),
            Some(root),
            "without a root bone the scene root is the only animation root"
        );
        success!()
    }

    /// The meshes of a first-person fixture.
    struct FirstPersonScene {
        world: World,
        state: VrmLoadState,
        body_node: Entity,
        split_body: Entity,
        face: Entity,
        arms: Entity,
        collar: Entity,
    }

    /// `root -> {Body (1, auto) -> split_body, Face (2) -> face,
    /// Arms (3, firstPersonOnly) -> 4 -> arms,
    /// Coat (5, thirdPersonOnly) -> Collar (6, both) -> collar}`.
    fn first_person_scene() -> FirstPersonScene {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let mesh = |world: &mut World, parent: Entity| {
            world
                .spawn((Mesh3d(Handle::default()), ChildOf(parent)))
                .id()
        };
        let body_node = world.spawn(ChildOf(root)).id();
        let split_body = mesh(&mut world, body_node);
        world
            .entity_mut(split_body)
            .insert(DynamicSkinnedMeshBounds);
        let face_node = world.spawn(ChildOf(root)).id();
        let face = mesh(&mut world, face_node);
        let arms_node = world.spawn(ChildOf(root)).id();
        let arms_child = world.spawn(ChildOf(arms_node)).id();
        let arms = mesh(&mut world, arms_child);
        let coat_node = world.spawn(ChildOf(root)).id();
        let collar_node = world.spawn(ChildOf(coat_node)).id();
        let collar = mesh(&mut world, collar_node);

        let mut state = state_with(vrm(r#"{
                "specVersion": "1.0",
                "humanoid": {"humanBones": {}},
                "firstPerson": {"meshAnnotations": [
                    {"node": 1, "firstPersonFlag": "auto"},
                    {"node": 3, "firstPersonFlag": "firstPersonOnly"},
                    {"node": 5, "firstPersonFlag": "thirdPersonOnly"},
                    {"node": 6, "firstPersonFlag": "both"}
                ]}
            }"#));
        state.node_entities = [
            (1, body_node),
            (2, face_node),
            (3, arms_node),
            (4, arms_child),
            (5, coat_node),
            (6, collar_node),
        ]
        .into_iter()
        .collect();
        state.primitive_entities = [((0, 0), vec![split_body]), ((1, 0), vec![face])]
            .into_iter()
            .collect();
        state.first_person.classes = [
            ((0, 0), AutoClass::Split),
            ((1, 0), AutoClass::ThirdPersonOnly),
        ]
        .into_iter()
        .collect();
        FirstPersonScene {
            world,
            state,
            body_node,
            split_body,
            face,
            arms,
            collar,
        }
    }

    /// First person is opt-in: the load records each mesh's view but leaves
    /// every mesh, the head copy included, on the default layer, so a plain
    /// camera renders the whole avatar.
    #[test]
    fn first_person_views_are_recorded_on_the_default_layer() -> TestResult {
        let FirstPersonScene {
            mut world,
            state,
            body_node,
            split_body,
            face,
            arms,
            collar,
        } = first_person_scene();
        let head_meshes = [((0, 0), Handle::default())].into_iter().collect();

        record_first_person_views(&state, &mut world);
        spawn_head_copies(&state, &head_meshes, &mut world);

        assert_eq!(
            world.query::<&RenderLayers>().iter(&world).count(),
            0,
            "no mesh leaves the default layer at load time"
        );
        let mut copies = world.query_filtered::<(Entity, &ChildOf), With<VrmHeadOnly>>();
        let copies: Vec<(Entity, Entity)> = copies
            .iter(&world)
            .map(|(copy, parent)| (copy, parent.parent()))
            .collect();
        let view = |entity: Entity| world.get::<FirstPersonView>(entity).copied();
        assert_eq!(view(split_body), None);
        assert_eq!(view(face), Some(FirstPersonView::ThirdPersonOnly));
        assert_eq!(view(arms), Some(FirstPersonView::FirstPersonOnly));
        assert_eq!(view(collar), None, "the nearest annotation wins");

        assert_eq!(copies.len(), 1);
        let (copy, parent) = copies[0];
        assert_eq!(parent, body_node, "the copy is a sibling of the body half");
        assert_eq!(view(copy), Some(FirstPersonView::ThirdPersonOnly));
        assert!(world.get::<DynamicSkinnedMeshBounds>(copy).is_some());
        success!()
    }

    /// Every node that instances a primitive gets its view and its head copy,
    /// not only the last one `bevy_gltf` spawned.
    #[test]
    fn every_instance_of_a_primitive_gets_its_first_person_view() -> TestResult {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let mut instance = || {
            let node = world.spawn(ChildOf(root)).id();
            let mesh = world.spawn((Mesh3d(Handle::default()), ChildOf(node))).id();
            (node, mesh)
        };
        let (left_node, left) = instance();
        let (right_node, right) = instance();
        let (_, face_a) = instance();
        let (_, face_b) = instance();
        let mut state = VrmLoadState {
            primitive_entities: [((0, 0), vec![left, right]), ((1, 0), vec![face_a, face_b])]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        state.first_person.classes = [
            ((0, 0), AutoClass::Split),
            ((1, 0), AutoClass::ThirdPersonOnly),
        ]
        .into_iter()
        .collect();
        let head_meshes = [((0, 0), Handle::default())].into_iter().collect();

        record_first_person_views(&state, &mut world);
        spawn_head_copies(&state, &head_meshes, &mut world);

        for face in [face_a, face_b] {
            assert_eq!(
                world.get::<FirstPersonView>(face).copied(),
                Some(FirstPersonView::ThirdPersonOnly)
            );
        }
        let parents: HashSet<Entity> = world
            .query_filtered::<&ChildOf, With<VrmHeadOnly>>()
            .iter(&world)
            .map(ChildOf::parent)
            .collect();
        assert_eq!(parents, HashSet::from_iter([left_node, right_node]));
        success!()
    }

    /// The humanoid bones of the fixture below, in the order its glTF node
    /// indices are numbered. The chain mirrors a real rig:
    /// `root -> root bone -> hips -> spine -> chest -> neck -> head`, with both
    /// eyes under the head — the four bones
    /// [`track_body_tracking`](crate::vrm::body_tracking) walks, plus the two
    /// eye bones [`track_looking_target`](crate::vrm::look_at::track_looking_target)
    /// needs.
    const GAZE_BONES: [&str; 7] = [
        "hips", "spine", "chest", "neck", "head", "leftEye", "rightEye",
    ];

    /// The entities of a pipeline scene's gaze rig.
    struct GazeScene {
        root: Entity,
        hips: Entity,
        spine: Entity,
        chest: Entity,
        neck: Entity,
        head: Entity,
        left_eye: Entity,
        right_eye: Entity,
    }

    impl GazeScene {
        /// The scene entities paired with the bone name each one carries.
        fn bones(&self) -> [(&'static str, Entity); 7] {
            [
                ("hips", self.hips),
                ("spine", self.spine),
                ("chest", self.chest),
                ("neck", self.neck),
                ("head", self.head),
                ("leftEye", self.left_eye),
                ("rightEye", self.right_eye),
            ]
        }
    }

    /// A scene the way `bevy_gltf` builds one: every node is an entity with a
    /// `Transform` and a `GlobalTransform`, and `on_gltf_node` has put the
    /// `VrmBone` on it (`nodes::process_node`).
    fn gaze_scene(app: &mut App) -> GazeScene {
        let world = app.world_mut();
        let root = world
            .spawn((Transform::default(), GlobalTransform::IDENTITY))
            .id();
        // The root bone is `setup_animation`'s animation root: the parent of
        // `hips`.
        let root_bone = world
            .spawn((
                Transform::default(),
                GlobalTransform::IDENTITY,
                ChildOf(root),
            ))
            .id();
        let mut parent = root_bone;
        let mut chain = Vec::with_capacity(GAZE_BONES.len());
        for name in GAZE_BONES {
            let entity = world
                .spawn((
                    Transform::default(),
                    GlobalTransform::IDENTITY,
                    VrmBone::from(name),
                    ChildOf(parent),
                ))
                .id();
            // The eyes hang off the head rather than continuing the chain.
            parent = if name == "head" { entity } else { parent };
            chain.push(entity);
        }
        // `chain` is exactly `GAZE_BONES` in order, so it is the node order.
        let mut chain = chain.into_iter();
        GazeScene {
            root,
            hips: chain.next().unwrap(),
            spine: chain.next().unwrap(),
            chest: chain.next().unwrap(),
            neck: chain.next().unwrap(),
            head: chain.next().unwrap(),
            left_eye: chain.next().unwrap(),
            right_eye: chain.next().unwrap(),
        }
    }

    /// The load state of [`gaze_scene`]: node index `i` is the entity of
    /// `GAZE_BONES[i]`, and the file declares the same humanoid map plus the
    /// `lookAt` the eyes are driven by.
    fn gaze_state(scene: &GazeScene) -> VrmLoadState {
        let human_bones = GAZE_BONES
            .iter()
            .enumerate()
            .map(|(index, name)| format!("\"{name}\": {{\"node\": {index}}}"))
            .collect::<Vec<_>>()
            .join(", ");
        let state = state_with(vrm(&format!(
            r#"{{
                "specVersion": "1.0",
                "humanoid": {{"humanBones": {{{human_bones}}}}},
                "lookAt": {{"offsetFromHeadBone": [0.0, 0.06, 0.0]}}
            }}"#
        )));
        VrmLoadState {
            bone_nodes: GAZE_BONES
                .iter()
                .enumerate()
                .map(|(index, name)| ((*name).to_owned(), index))
                .collect(),
            node_entities: scene
                .bones()
                .into_iter()
                .enumerate()
                .map(|(index, (_, entity))| (index, entity))
                .collect(),
            ..state
        }
    }

    /// Every step of [`finalize`] a gaze target depends on, in order.
    ///
    /// `finalize` itself is not callable here: it needs a `LoadContext`, and the
    /// first-person pass inside it asks that context for a mesh handle. The steps
    /// are the same functions `finalize` calls, which is what makes these tests
    /// about the pipeline and not about a hand-written approximation of it.
    fn finalize_for_gaze(
        app: &mut App,
        state: &VrmLoadState,
        root: Entity,
    ) {
        insert_rest_transforms(app.world_mut(), root);
        build_look_at(state, app.world_mut(), root);
        setup_animation(state, app.world_mut(), root);
        insert_humanoid_bone_holders(app.world_mut(), root);
        mark_initialized(app.world_mut(), root);
    }

    /// The gaze systems address the bones *through* the avatar root, so without
    /// the holders a pipeline scene's `LookAt` and `BodyTracking` do nothing at
    /// all — the root carries the bone entities, not the bones.
    #[test]
    fn bone_holders_reach_the_scene_root() -> TestResult {
        let mut app = test_app();
        let scene = gaze_scene(&mut app);
        let state = gaze_state(&scene);

        finalize_for_gaze(&mut app, &state, scene.root);

        let world = app.world();
        // The holders the two gaze systems read.
        assert_eq!(
            world.get::<HeadBoneEntity>(scene.root).map(|head| head.0),
            Some(scene.head),
            "`track_looking_target` and `track_body_tracking` need the head"
        );
        assert_eq!(
            world.get::<LeftEyeBoneEntity>(scene.root).map(|eye| eye.0),
            Some(scene.left_eye)
        );
        assert_eq!(
            world.get::<RightEyeBoneEntity>(scene.root).map(|eye| eye.0),
            Some(scene.right_eye)
        );
        assert_eq!(
            world.get::<NeckBoneEntity>(scene.root).map(|bone| bone.0),
            Some(scene.neck),
            "body tracking's optional chain bones resolve too"
        );
        assert_eq!(
            world.get::<ChestBoneEntity>(scene.root).map(|bone| bone.0),
            Some(scene.chest)
        );
        assert_eq!(
            world.get::<SpineBoneEntity>(scene.root).map(|bone| bone.0),
            Some(scene.spine)
        );
        assert_eq!(
            world.get::<HipsBoneEntity>(scene.root).map(|bone| bone.0),
            Some(scene.hips)
        );
        // A holder belongs to the root, not to a bone.
        assert!(world.get::<HeadBoneEntity>(scene.hips).is_none());

        // The bone's own marker rides on the bone.
        assert!(world.get::<Head>(scene.head).is_some());
        assert!(world.get::<LeftEye>(scene.left_eye).is_some());
        assert!(world.get::<Hips>(scene.hips).is_some());
        success!()
    }

    /// The whole point of the holders: the two gaze systems stop matching
    /// nothing. The plugins are the crate's own, so the systems under test are
    /// the scheduled ones rather than a hand-run copy of them, and
    /// `BodyTracking`'s own observer supplies `SmoothedGaze`.
    ///
    /// Smoothing is switched off so the assertion does not depend on how long
    /// the first frame's delta happens to be.
    #[test]
    fn gaze_control_is_no_longer_inert_on_a_pipeline_scene() -> TestResult {
        let mut app = test_app();
        app.add_plugins((LookAtPlugin, BodyTrackingPlugin));
        let scene = gaze_scene(&mut app);
        let state = gaze_state(&scene);

        finalize_for_gaze(&mut app, &state, scene.root);
        // `finalize` bakes only the private `VrmSceneRoot`; the public `Vrm`
        // marker is written by the attach stage (`vrm::instance`), which this
        // pipeline-level helper does not run. Body tracking reads the root
        // `GlobalTransform` through `With<Vrm>`, so the fixture must supply it.
        app.world_mut().entity_mut(scene.root).insert(Vrm);

        // Off to the right, slightly below the head: both a yaw and a pitch.
        let target = app
            .world_mut()
            .spawn((
                Transform::default(),
                GlobalTransform::from_xyz(1.0, 0.5, 1.0),
            ))
            .id();
        app.world_mut().entity_mut(scene.root).insert((
            LookAt::Target(target),
            BodyTracking {
                smoothing: 0.0,
                output_smoothing: 0.0,
                ..Default::default()
            },
        ));

        app.update();

        let world = app.world();
        // `track_looking_target` drove the eyes.
        assert!(
            turned(world, scene.left_eye),
            "the left eye did not turn towards the target"
        );
        assert!(
            turned(world, scene.right_eye),
            "the right eye did not turn towards the target"
        );
        // `track_body_tracking` drove the chain, which is only reachable through
        // `HeadBoneEntity` + the optional `Neck` / `Chest` / `Spine` holders.
        for bone in [scene.spine, scene.chest, scene.neck, scene.head] {
            assert!(turned(world, bone), "a body-tracking bone did not turn");
        }
        // The avatar root itself is not a gaze bone.
        assert!(!turned(world, scene.root));
        success!()
    }

    /// Whether an entity's rotation has left the identity at all.
    fn turned(
        world: &World,
        entity: Entity,
    ) -> bool {
        world
            .get::<Transform>(entity)
            .is_some_and(|transform| transform.rotation.angle_between(Quat::IDENTITY) > 1e-3)
    }

    /// The source path of the file *is* observable at load time, so a scene
    /// carries `VrmPath` and `RequestDetachVrm` has something to remove.
    ///
    /// What is pinned here is the observable `insert_source_path` reads, because
    /// the function itself cannot be called from a test: `LoadContext::new` is
    /// `pub(crate)` inside `bevy_asset`, and `finalize` needs a context for its
    /// first-person pass besides. `AssetServer::load` builds the
    /// `LoadContext` from exactly this [`AssetPath`]
    /// (`server/mod.rs`), and `bevy_gltf` derives a scene's context
    /// from the file's with `begin_labeled_asset`
    /// (`loader/mod.rs`), which clones the path and attaches the `Scene0`
    /// label only when the asset is registered.
    #[test]
    fn the_source_path_is_observable_on_a_scene_load_context() -> TestResult {
        let loaded = bevy::asset::AssetPath::from("vrm/Elmer.vrm");

        assert_eq!(
            loaded.path(),
            std::path::Path::new("vrm/Elmer.vrm"),
            "`LoadContext::path` of a scene is the file's own path, not `Scene0` of it"
        );
        assert_eq!(
            VrmPath::new(loaded.path()).0,
            VrmPath::new("vrm/Elmer.vrm").0,
            "and it is exactly the path the caller passed to `AssetServer::load`"
        );
        // The same holds for the labeled handle a caller spawns the scene from.
        let labeled = bevy::asset::AssetPath::from("vrm/Elmer.vrm#Scene0");
        assert_eq!(labeled.path(), loaded.path());
        success!()
    }
}
