mod update;

use crate::prelude::ColliderShape;
use crate::vrm::spring_bone::update::SpringBoneUpdatePlugin;
use bevy::app::App;
use bevy::ecs::entity::MapEntities;
use bevy::math::{Mat4, Quat, Vec3};
use bevy::prelude::*;

/// The component that holds the spring bone state of each Joint
///
/// Implement the method described in the  [Official documentation](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_springBone-1.0/README.ja.md#%E5%88%9D%E6%9C%9F%E5%8C%96)
#[derive(PartialEq, Debug, Clone, Default, Reflect, Component)]
#[reflect(Default, Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) struct SpringJointState {
    prev_tail: Vec3,
    current_tail: Vec3,
    bone_axis: Vec3,
    bone_length: f32,
    initial_local_matrix: Mat4,
    initial_local_rotation: Quat,
}

impl SpringJointState {
    /// The simulation state of one joint at its rest pose.
    ///
    /// The tail is left unseeded: it is a world (or center-space) position,
    /// which depends on where the avatar is placed, so the first simulated
    /// frame seeds it from the live pose (see [`Self::seed_tail`]).
    pub(crate) fn from_rest(
        bone_axis: Vec3,
        bone_length: f32,
        rest: Transform,
    ) -> Self {
        Self {
            prev_tail: Vec3::ZERO,
            current_tail: Vec3::ZERO,
            bone_axis,
            bone_length,
            initial_local_matrix: rest.to_matrix(),
            initial_local_rotation: rest.rotation,
        }
    }

    /// Places the tail at `tail` with no velocity.
    pub(crate) fn seed_tail(
        &mut self,
        tail: Vec3,
    ) {
        self.prev_tail = tail;
        self.current_tail = tail;
    }

    #[cfg(test)]
    pub(crate) fn tails(&self) -> (Vec3, Vec3) {
        (self.prev_tail, self.current_tail)
    }

    #[cfg(test)]
    pub(crate) fn set_tails(
        &mut self,
        prev_tail: Vec3,
        current_tail: Vec3,
    ) {
        self.prev_tail = prev_tail;
        self.current_tail = current_tail;
    }
}

/// Reseeds every spring joint under `entity`, including the entity itself.
///
/// Each joint takes the live position of its tail node, in center space when
/// its spring root has a center node. This is the same seeding used when a
/// joint state is added, so a teleport or pose switch starts with no inertia
/// pointing at the previous pose.
pub(crate) fn reseed_spring_bones(
    entity: Entity,
    spring_roots: &Query<&SpringRoot>,
    joint_states: &mut Query<&mut SpringJointState>,
    transforms: &Query<&GlobalTransform>,
    children: &Query<&Children>,
) {
    if let Ok(root) = spring_roots.get(entity) {
        let center_gtf = root
            .center_node
            .and_then(|center| transforms.get(center).ok())
            .copied();
        for (index, &joint) in root.joints.iter().enumerate() {
            let (Ok(mut state), Some(&tail)) =
                (joint_states.get_mut(joint), root.joints.get(index + 1))
            else {
                continue;
            };
            let Ok(tail_gtf) = transforms.get(tail) else {
                continue;
            };
            state.seed_tail(global_to_center_local(tail_gtf.translation(), &center_gtf));
        }
    }
    let Ok(entity_children) = children.get(entity) else {
        return;
    };
    for child in entity_children.iter() {
        reseed_spring_bones(child, spring_roots, joint_states, transforms, children);
    }
}

/// Converts a world tail position into the spring root's center space.
pub(crate) fn global_to_center_local(
    tail_pos: Vec3,
    center_gtf: &Option<GlobalTransform>,
) -> Vec3 {
    if let Some(gtf) = center_gtf.as_ref() {
        gtf.to_matrix().inverse().transform_point3(tail_pos)
    } else {
        tail_pos
    }
}

/// Queues the target entity for a spring reseed under its subtree.
///
/// Queued roots are reseeded once per frame in `PostUpdate`, after
/// `VrmSystemSets::PropagateAfterExpressions` and immediately before
/// `VrmSystemSets::SpringBone`, so the tails are placed at their live positions
/// using this frame's propagated transforms, with inertia cleared. Repeated
/// triggers for the same entity in one frame share a single reseed.
///
/// Trigger this after teleporting a rig or switching its pose outside VRMA
/// playback. It is a no-op for an entity without spring joints in its subtree.
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy_vrm1::prelude::*;
/// fn teleport(mut commands: Commands, vrm: Entity) {
///     commands.entity(vrm).trigger(ReseedSpringBones);
/// }
/// ```
#[derive(EntityEvent, Debug, Reflect)]
pub struct ReseedSpringBones(pub Entity);

#[derive(Resource, Debug, Default)]
pub(crate) struct ReseedQueue(pub Vec<Entity>);

impl ReseedQueue {
    pub(crate) fn push(
        &mut self,
        entity: Entity,
    ) {
        if !self.0.contains(&entity) {
            self.0.push(entity);
        }
    }
}

#[derive(Component, Debug, Clone, PartialEq, Default, Reflect)]
#[reflect(Component, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringRoot {
    /// Represents a list of entity of spring joints belonging to the spring chain.
    /// This component is inserted into the root entity of the chain.
    #[entities]
    pub joints: SpringJoints,

    #[entities]
    pub colliders: SpringColliders,

    /// If the spring chain has a center node,
    /// The inertia of the spring bone is evaluated in the [`Center Space`](https://github.com/vrm-c/vrm-specification/tree/master/specification/VRMC_springBone-1.0#center-space).
    #[entities]
    pub center_node: SpringCenterNode,
}

#[derive(Eq, PartialEq, Debug, Clone, Default, Deref, Reflect, MapEntities)]
#[reflect(Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringJoints(#[entities] pub Vec<Entity>);

#[derive(PartialEq, Debug, Clone, Default, Deref, Reflect)]
#[reflect(Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringColliders(pub Vec<(Entity, ColliderShape)>);

impl MapEntities for SpringColliders {
    fn map_entities<M: EntityMapper>(
        &mut self,
        entity_mapper: &mut M,
    ) {
        for (collider, _) in &mut self.0 {
            collider.map_entities(entity_mapper);
        }
    }
}

#[derive(Eq, PartialEq, Debug, Clone, Default, Deref, Reflect, MapEntities)]
#[reflect(Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringCenterNode(#[entities] pub Option<Entity>);

#[derive(Component, Debug, Copy, Clone, Default, PartialEq, Reflect)]
#[reflect(Default, Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct SpringJointProps {
    pub drag_force: f32,
    pub gravity_dir: Vec3,
    pub gravity_power: f32,
    pub hit_radius: f32,
    pub stiffness: f32,
}
pub struct VrmSpringBonePlugin;

impl Plugin for VrmSpringBonePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<SpringRoot>()
            .register_type::<SpringJointState>()
            .register_type::<SpringJoints>()
            .register_type::<SpringColliders>()
            .register_type::<SpringCenterNode>()
            .register_type::<SpringJointProps>()
            .register_type::<ReseedSpringBones>()
            .add_observer(apply_reseed_spring_bones)
            .add_plugins(SpringBoneUpdatePlugin);
    }
}

fn apply_reseed_spring_bones(
    trigger: On<ReseedSpringBones>,
    mut queue: ResMut<ReseedQueue>,
) {
    queue.push(trigger.event_target());
}

pub(crate) fn drain_reseed_queue(
    mut queue: ResMut<ReseedQueue>,
    spring_roots: Query<&SpringRoot>,
    mut joint_states: Query<&mut SpringJointState>,
    transforms: Query<&GlobalTransform>,
    children: Query<&Children>,
) {
    for root in queue.0.drain(..) {
        reseed_spring_bones(
            root,
            &spring_roots,
            &mut joint_states,
            &transforms,
            &children,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_set::VrmSystemSets;
    use crate::tests::test_app;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::time::TimeUpdateStrategy;
    use bevy::transform::TransformSystems;
    use std::f32::consts::FRAC_PI_2;
    use std::time::Duration;

    struct ReseedStrand {
        app: App,
        root: Entity,
        head: Entity,
        tail: Entity,
    }

    fn reseed_strand(with_center: bool) -> ReseedStrand {
        let mut app = test_app();
        app.add_plugins((bevy::transform::TransformPlugin, VrmSpringBonePlugin));
        app.configure_sets(
            PostUpdate,
            VrmSystemSets::PropagateAfterExpressions.after(TransformSystems::Propagate),
        );
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )));
        app.finish();
        app.cleanup();
        let world = app.world_mut();
        let root = world
            .spawn((Transform::IDENTITY, GlobalTransform::IDENTITY))
            .id();
        let head = world
            .spawn((
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
                ChildOf(root),
            ))
            .id();
        let tail = world
            .spawn((
                Transform::from_xyz(0.0, 1.0, 0.0),
                GlobalTransform::from_xyz(0.0, 1.0, 0.0),
                ChildOf(head),
            ))
            .id();
        let center = with_center.then(|| {
            let transform = Transform {
                translation: Vec3::new(100.0, 0.0, 0.0),
                rotation: Quat::from_rotation_z(FRAC_PI_2),
                ..default()
            };
            world
                .spawn((transform, GlobalTransform::from(transform)))
                .id()
        });
        let props = SpringJointProps {
            drag_force: 0.0,
            gravity_dir: Vec3::NEG_X,
            gravity_power: 50.0,
            hit_radius: 0.0,
            stiffness: 0.0,
        };
        world.entity_mut(head).insert((
            props,
            SpringJointState::from_rest(Vec3::Y, 1.0, Transform::IDENTITY),
            SpringRoot {
                joints: SpringJoints(vec![head, tail]),
                center_node: SpringCenterNode(center),
                ..default()
            },
        ));
        world.entity_mut(tail).insert(props);
        ReseedStrand {
            app,
            root,
            head,
            tail,
        }
    }

    fn joint_state(strand: &ReseedStrand) -> &SpringJointState {
        strand
            .app
            .world()
            .get::<SpringJointState>(strand.head)
            .expect("the head keeps its spring state")
    }

    fn drain(strand: &mut ReseedStrand) {
        strand
            .app
            .world_mut()
            .run_system_once(drain_reseed_queue)
            .expect("the reseed drain runs");
    }

    fn teleport_root(strand: &mut ReseedStrand) {
        let world = strand.app.world_mut();
        world.entity_mut(strand.root).insert((
            Transform::from_xyz(100.0, 0.0, 0.0),
            GlobalTransform::from_xyz(100.0, 0.0, 0.0),
        ));
        world
            .entity_mut(strand.head)
            .insert(GlobalTransform::from_xyz(100.0, 0.0, 0.0));
        world
            .entity_mut(strand.tail)
            .insert(GlobalTransform::from_xyz(100.0, 1.0, 0.0));
    }

    /// A teleport must not leave the joint simulating against the old pose:
    /// without the trigger the stale position and inertia survive the jump.
    #[test]
    fn a_reseed_after_a_teleport_puts_the_joint_at_the_live_tail() {
        let mut reseeded = reseed_strand(false);
        let mut stale = reseed_strand(false);
        for strand in [&mut reseeded, &mut stale] {
            strand.app.update();
            strand.app.update();
            teleport_root(strand);
        }

        reseeded
            .app
            .world_mut()
            .trigger(ReseedSpringBones(reseeded.root));
        drain(&mut reseeded);

        let state = joint_state(&reseeded);
        assert!(
            state
                .current_tail
                .abs_diff_eq(Vec3::new(100.0, 1.0, 0.0), 1e-5),
            "the joint must start at the live tail: {:?}",
            state.current_tail
        );
        assert_eq!(
            state.prev_tail, state.current_tail,
            "reseeding must clear the inertia"
        );
        let stale_state = joint_state(&stale);
        assert!(
            stale_state.current_tail.x.abs() < 1.0,
            "without the trigger the state stays at the old pose: {:?}",
            stale_state.current_tail
        );
        assert_ne!(
            stale_state.prev_tail, stale_state.current_tail,
            "without the trigger the old inertia survives"
        );

        reseeded.app.update();
        let tail = reseeded
            .app
            .world()
            .get::<GlobalTransform>(reseeded.tail)
            .unwrap()
            .translation();
        assert!(
            (tail.x - 100.0).abs() < 1.0 && tail.y > 0.5,
            "the next spring step must start from the live tail: {tail:?}"
        );
    }

    #[test]
    fn a_reseed_uses_center_local_tail_positions() {
        let mut strand = reseed_strand(true);
        strand.app.update();
        strand
            .app
            .world_mut()
            .entity_mut(strand.tail)
            .insert(GlobalTransform::from_xyz(100.0, 1.0, 0.0));

        strand
            .app
            .world_mut()
            .trigger(ReseedSpringBones(strand.root));
        drain(&mut strand);

        let state = joint_state(&strand);
        assert!(
            state.current_tail.abs_diff_eq(Vec3::X, 1e-5),
            "the tail must be stored in center space: {:?}",
            state.current_tail
        );
        assert_eq!(state.prev_tail, state.current_tail);
    }

    /// Seeding must see the pose this frame's propagation produces, not the
    /// one left over from the previous frame.
    #[test]
    fn a_reseed_in_the_teleport_frame_seeds_from_the_new_pose() {
        let mut strand = reseed_strand(false);
        let neutral = SpringJointProps {
            gravity_power: 0.0,
            ..*strand
                .app
                .world()
                .get::<SpringJointProps>(strand.head)
                .unwrap()
        };
        let world = strand.app.world_mut();
        world.entity_mut(strand.head).insert(neutral);
        world.entity_mut(strand.tail).insert(neutral);
        strand.app.update();
        strand.app.update();
        strand
            .app
            .world_mut()
            .entity_mut(strand.root)
            .insert(Transform::from_xyz(100.0, 0.0, 0.0));
        strand
            .app
            .world_mut()
            .trigger(ReseedSpringBones(strand.root));

        strand.app.update();

        let state = joint_state(&strand);
        assert!(
            state
                .current_tail
                .abs_diff_eq(Vec3::new(100.0, 1.0, 0.0), 1e-5),
            "the seed must use this frame's tail: {:?}",
            state.current_tail
        );
        assert_eq!(state.prev_tail, state.current_tail);
        assert!(strand.app.world().resource::<ReseedQueue>().0.is_empty());
    }

    #[test]
    fn repeated_triggers_in_one_frame_enqueue_one_walk() {
        let mut strand = reseed_strand(false);
        drain(&mut strand);
        strand
            .app
            .world_mut()
            .get_mut::<SpringJointState>(strand.head)
            .unwrap()
            .set_tails(Vec3::X, Vec3::ZERO);
        let world = strand.app.world_mut();
        world.trigger(ReseedSpringBones(strand.root));
        world.trigger(ReseedSpringBones(strand.root));

        assert_eq!(world.resource::<ReseedQueue>().0, vec![strand.root]);
        drain(&mut strand);

        assert!(strand.app.world().resource::<ReseedQueue>().0.is_empty());
        let state = joint_state(&strand);
        assert!(state.current_tail.abs_diff_eq(Vec3::Y, 1e-5));
        assert_eq!(state.prev_tail, state.current_tail);
    }

    #[test]
    fn a_reseed_trigger_on_an_entity_without_springs_is_a_no_op() {
        let mut app = test_app();
        app.add_plugins(VrmSpringBonePlugin);
        let entity = app.world_mut().spawn_empty().id();

        app.world_mut().trigger(ReseedSpringBones(entity));
        app.world_mut()
            .run_system_once(drain_reseed_queue)
            .expect("the reseed drain runs");

        assert!(app.world().resource::<ReseedQueue>().0.is_empty());
        assert!(app.world().get_entity(entity).is_ok());
    }
}
