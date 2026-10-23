//! Provides support for `VRMC_vrm.firstPerson`.
//!
//! The meshes are classified while the `.vrm` loads, and `auto` meshes are
//! split there into a body part and a head part. Every mesh stays on the
//! default layer until [`RequestEnableFirstPerson`] applies:
//!
//! | firstPersonFlag | RenderLayers |
//! |---|---|
//! | `both` | 0 (default) |
//! | `thirdPersonOnly` | `FirstPersonLayers::third_person_only` |
//! | `firstPersonOnly` | `FirstPersonLayers::first_person_only` |
//! | `auto` | mesh is split: head part -> thirdPersonOnly, the rest -> both |

use crate::vrm::Vrm;
use crate::vrm::components::FirstPersonView;
use bevy::app::{App, Plugin};
use bevy::camera::visibility::{Layer, RenderLayers};
use bevy::ecs::lifecycle::Add;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::{
    component::Component,
    entity::Entity,
    event::EntityEvent,
    hierarchy::Children,
    observer::On,
    query::With,
    resource::Resource,
    system::{Commands, Query, Res},
};
use bevy::mesh::Mesh3d;
use bevy::reflect::Reflect;

pub(crate) struct VrmFirstPersonPlugin;

impl Plugin for VrmFirstPersonPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.init_resource::<FirstPersonLayers>()
            .register_type::<FirstPersonCamera>()
            .register_type::<ThirdPersonCamera>()
            .add_observer(setup_first_person_camera)
            .add_observer(setup_third_person_camera)
            .add_observer(apply_enable_first_person)
            .add_observer(apply_disable_first_person);
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

fn apply_enable_first_person(
    trigger: On<RequestEnableFirstPerson>,
    mut commands: Commands,
    vrms: Query<(), With<Vrm>>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    views: Query<&FirstPersonView>,
    layers: Res<FirstPersonLayers>,
) {
    let vrm = trigger.event_target();
    if !vrms.contains(vrm) {
        return;
    }
    for mesh_entity in descendants_with_mesh(vrm, &children, &meshes) {
        let render_layers = match views.get(mesh_entity) {
            Ok(FirstPersonView::ThirdPersonOnly) => RenderLayers::layer(layers.third_person_only),
            Ok(FirstPersonView::FirstPersonOnly) => RenderLayers::layer(layers.first_person_only),
            Err(_) => RenderLayers::default(),
        };
        commands.entity(mesh_entity).insert(render_layers);
    }
}

fn apply_disable_first_person(
    trigger: On<RequestDisableFirstPerson>,
    mut commands: Commands,
    vrms: Query<(), With<Vrm>>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
) {
    let vrm = trigger.event_target();
    if !vrms.contains(vrm) {
        return;
    }
    for mesh_entity in descendants_with_mesh(vrm, &children, &meshes) {
        commands.entity(mesh_entity).insert(RenderLayers::default());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::VrmHeadOnly;
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use bevy::asset::Handle;
    use bevy::ecs::hierarchy::ChildOf;

    struct Avatar {
        vrm: Entity,
        body: Entity,
        head_copy: Entity,
        first_person_only: Entity,
    }

    /// The shape the loader leaves behind: a body half on the default layer,
    /// its head copy and a `firstPersonOnly` mesh, all below the VRM root.
    fn spawn_avatar(app: &mut App) -> Avatar {
        let world = app.world_mut();
        let vrm = world.spawn(Vrm).id();
        let node = world.spawn(ChildOf(vrm)).id();
        let body = world.spawn((Mesh3d(Handle::default()), ChildOf(node))).id();
        let head_copy = world
            .spawn((
                Mesh3d(Handle::default()),
                VrmHeadOnly,
                FirstPersonView::ThirdPersonOnly,
                ChildOf(node),
            ))
            .id();
        let first_person_only = world
            .spawn((
                Mesh3d(Handle::default()),
                FirstPersonView::FirstPersonOnly,
                ChildOf(vrm),
            ))
            .id();
        Avatar {
            vrm,
            body,
            head_copy,
            first_person_only,
        }
    }

    fn app() -> App {
        let mut app = test_app();
        app.add_plugins(VrmFirstPersonPlugin);
        app
    }

    fn layers(
        app: &App,
        entity: Entity,
    ) -> Option<RenderLayers> {
        app.world().get::<RenderLayers>(entity).cloned()
    }

    /// The loaded avatar renders whole on the default layer until first person
    /// is enabled; enabling applies the views recorded at load time, and
    /// disabling puts every mesh back on the default layer.
    #[test]
    fn enabling_applies_the_recorded_views_and_disabling_reverts_them() -> TestResult {
        let mut app = app();
        let avatar = spawn_avatar(&mut app);
        for mesh in [avatar.body, avatar.head_copy, avatar.first_person_only] {
            assert_eq!(layers(&app, mesh), None);
        }

        app.world_mut()
            .trigger(RequestEnableFirstPerson(avatar.vrm));
        app.update();
        let default_layers = FirstPersonLayers::default();
        assert_eq!(layers(&app, avatar.body), Some(RenderLayers::default()));
        assert_eq!(
            layers(&app, avatar.head_copy),
            Some(RenderLayers::layer(default_layers.third_person_only))
        );
        assert_eq!(
            layers(&app, avatar.first_person_only),
            Some(RenderLayers::layer(default_layers.first_person_only))
        );

        app.world_mut()
            .trigger(RequestDisableFirstPerson(avatar.vrm));
        app.update();
        for mesh in [avatar.body, avatar.head_copy, avatar.first_person_only] {
            assert_eq!(layers(&app, mesh), Some(RenderLayers::default()));
        }
        success!()
    }

    #[test]
    fn enabling_reads_the_configured_layers() -> TestResult {
        let mut app = app();
        app.insert_resource(FirstPersonLayers {
            first_person_only: 3,
            third_person_only: 4,
        });
        let avatar = spawn_avatar(&mut app);

        app.world_mut()
            .trigger(RequestEnableFirstPerson(avatar.vrm));
        app.update();

        assert_eq!(layers(&app, avatar.head_copy), Some(RenderLayers::layer(4)));
        assert_eq!(
            layers(&app, avatar.first_person_only),
            Some(RenderLayers::layer(3))
        );
        success!()
    }

    #[test]
    fn a_target_that_is_not_a_vrm_is_left_alone() -> TestResult {
        let mut app = app();
        let owner = app.world_mut().spawn_empty().id();
        let mesh = app
            .world_mut()
            .spawn((Mesh3d(Handle::default()), ChildOf(owner)))
            .id();

        app.world_mut().trigger(RequestEnableFirstPerson(owner));
        app.world_mut().trigger(RequestDisableFirstPerson(owner));
        app.update();

        assert_eq!(layers(&app, mesh), None);
        success!()
    }

    #[test]
    fn cameras_see_the_default_layer_and_their_own_view() -> TestResult {
        let mut app = app();
        let first_person = app.world_mut().spawn(FirstPersonCamera).id();
        let third_person = app.world_mut().spawn(ThirdPersonCamera).id();
        app.update();

        let default_layers = FirstPersonLayers::default();
        assert_eq!(
            layers(&app, first_person),
            Some(RenderLayers::default().with(default_layers.first_person_only))
        );
        assert_eq!(
            layers(&app, third_person),
            Some(RenderLayers::default().with(default_layers.third_person_only))
        );
        success!()
    }
}
