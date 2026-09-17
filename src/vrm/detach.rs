use crate::vrm::body_tracking::{BodyTracking, SmoothedGaze};
use crate::vrm::expressions::{ExpressionEntityMap, VrmExpressionRegistry};
use crate::vrm::first_person::FirstPersonRegistry;
use crate::vrm::gltf::extensions::vrmc_vrm::LookAtProperties;
use crate::vrm::humanoid_bone::HumanoidBoneRegistry;
use crate::vrm::loader::VrmHandle;
use crate::vrm::look_at::LookAt;
use crate::vrm::mtoon::VrmcMaterialRegistry;
use crate::vrm::node_constraint::registry::NodeConstraintRegistry;
use crate::vrm::spring_bone::registry::{
    SpringColliderRegistry, SpringJointPropsRegistry, SpringNodeRegistry,
};
use crate::vrm::{
    Initialized, RestGlobalTransform, RestTransform, RestWorldTransform, Vrm, VrmPath,
};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot};

/// Triggers VRM detachment on the target entity.
///
/// Removes all VRM-related components and despawns the child hierarchy,
/// leaving the root entity itself alive.
///
/// # Usage
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy_vrm1::vrm::detach::RequestDetachVrm;
/// fn detach(mut commands: Commands, vrm_entity: Entity) {
///     commands.entity(vrm_entity).trigger(RequestDetachVrm);
/// }
/// ```
#[derive(EntityEvent)]
pub struct RequestDetachVrm(pub Entity);

pub(crate) struct VrmDetachPlugin;

impl Plugin for VrmDetachPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_observer(apply_detach_vrm);
    }
}

fn apply_detach_vrm(
    trigger: On<RequestDetachVrm>,
    mut commands: Commands,
    children_query: Query<&Children>,
    vrm_check: Query<(), Or<(With<Vrm>, With<VrmHandle>)>>,
) {
    let entity = trigger.event_target();

    if vrm_check.get(entity).is_err() {
        return;
    }

    remove_vrm_components(&mut commands, entity);
    despawn_children(&mut commands, entity, &children_query);
}

/// Removes all VRM-related components from the entity.
fn remove_vrm_components(
    commands: &mut Commands,
    entity: Entity,
) {
    commands
        .entity(entity)
        // Core
        .try_remove::<Vrm>()
        .try_remove::<VrmPath>()
        .try_remove::<Initialized>()
        .try_remove::<VrmHandle>()
        .try_remove::<Name>()
        .try_remove::<WorldAssetRoot>()
        // Rest transforms
        .try_remove::<RestTransform>()
        .try_remove::<RestGlobalTransform>()
        .try_remove::<RestWorldTransform>()
        // Registries (pub)
        .try_remove::<VrmcMaterialRegistry>()
        .try_remove::<NodeConstraintRegistry>()
        .try_remove::<ExpressionEntityMap>()
        // Registries (pub(crate))
        .try_remove::<VrmExpressionRegistry>()
        .try_remove::<HumanoidBoneRegistry>()
        .try_remove::<SpringJointPropsRegistry>()
        .try_remove::<SpringColliderRegistry>()
        .try_remove::<SpringNodeRegistry>()
        .try_remove::<FirstPersonRegistry>()
        // Gaze/Body
        .try_remove::<LookAtProperties>()
        .try_remove::<LookAt>()
        .try_remove::<BodyTracking>()
        .try_remove::<SmoothedGaze>();

    remove_bone_entities(commands, entity);
}

macro_rules! remove_bone_entities {
    ($cmd:expr, $entity:expr, $($bone:ident),+ $(,)?) => {
        paste::paste! {
            $cmd.entity($entity)
                $(.try_remove::<crate::vrm::humanoid_bone::prelude::[<$bone BoneEntity>]>())+;
        }
    };
}

/// Removes all 55 bone entity holder components.
fn remove_bone_entities(
    commands: &mut Commands,
    entity: Entity,
) {
    remove_bone_entities!(
        commands,
        entity,
        Hips,
        RightRingProximal,
        RightThumbDistal,
        RightRingIntermediate,
        RightUpperArm,
        LeftIndexProximal,
        LeftUpperLeg,
        LeftFoot,
        LeftIndexDistal,
        LeftThumbMetacarpal,
        RightLowerArm,
        LeftMiddleDistal,
        RightUpperLeg,
        LeftToes,
        LeftThumbDistal,
        RightShoulder,
        RightThumbMetacarpal,
        Spine,
        LeftLowerLeg,
        LeftShoulder,
        LeftUpperArm,
        UpperChest,
        RightToes,
        RightIndexDistal,
        LeftMiddleProximal,
        LeftRingProximal,
        LeftRingDistal,
        LeftThumbProximal,
        LeftIndexIntermediate,
        LeftLittleProximal,
        LeftLittleDistal,
        RightHand,
        RightLittleProximal,
        LeftRingIntermediate,
        RightIndexIntermediate,
        Chest,
        LeftHand,
        RightLittleIntermediate,
        RightFoot,
        RightLowerLeg,
        LeftLittleIntermediate,
        LeftLowerArm,
        RightLittleDistal,
        RightMiddleIntermediate,
        RightMiddleProximal,
        RightThumbProximal,
        Neck,
        Jaw,
        Head,
        LeftEye,
        RightEye,
        LeftMiddleIntermediate,
        RightRingDistal,
        RightIndexProximal,
        RightMiddleDistal,
    );
}

/// Despawns all direct children of the entity.
///
/// Bevy 0.18's `despawn()` is recursive, so this also despawns all descendants.
fn despawn_children(
    commands: &mut Commands,
    entity: Entity,
    children_query: &Query<&Children>,
) {
    let Ok(children) = children_query.get(entity) else {
        return;
    };
    for child in children.iter() {
        commands.entity(child).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::platform::collections::HashMap;

    fn setup_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(VrmDetachPlugin);
        app
    }

    #[test]
    fn test_detach_removes_vrm_components() {
        let mut app = setup_app();

        let vrm_entity = app
            .world_mut()
            .spawn((
                Vrm,
                Initialized,
                ExpressionEntityMap(HashMap::default()),
                RestWorldTransform::default(),
            ))
            .id();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        let world = app.world();
        assert!(!world.entity(vrm_entity).contains::<Vrm>());
        assert!(!world.entity(vrm_entity).contains::<Initialized>());
        assert!(!world.entity(vrm_entity).contains::<ExpressionEntityMap>());
        assert!(!world.entity(vrm_entity).contains::<RestWorldTransform>());
        // Entity itself survives
        assert!(world.get_entity(vrm_entity).is_ok());
    }

    #[test]
    fn test_detach_despawns_children() {
        let mut app = setup_app();

        let child = app.world_mut().spawn_empty().id();
        let vrm_entity = app.world_mut().spawn(Vrm).id();
        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .add_child(child);
        app.update();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        // Root survives
        assert!(app.world().get_entity(vrm_entity).is_ok());
        // Child is despawned
        assert!(app.world().get_entity(child).is_err());
    }

    #[test]
    fn test_detach_on_non_vrm_entity() {
        let mut app = setup_app();

        let child = app.world_mut().spawn_empty().id();
        let entity = app.world_mut().spawn(Name::new("not-a-vrm")).id();
        app.world_mut().commands().entity(entity).add_child(child);
        app.update();

        app.world_mut()
            .commands()
            .entity(entity)
            .trigger(RequestDetachVrm);
        app.update();

        // Name and children should remain since this isn't a VRM entity
        assert!(app.world().entity(entity).contains::<Name>());
        assert!(app.world().get_entity(child).is_ok());
    }

    #[test]
    fn test_detach_idempotent() {
        let mut app = setup_app();

        let vrm_entity = app.world_mut().spawn(Vrm).id();

        // First detach
        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        // Second detach — should not panic
        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        assert!(app.world().get_entity(vrm_entity).is_ok());
    }
}
