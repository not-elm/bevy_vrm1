//! You can transition to the corresponding animation by pressing the number keys 1 to 4.
//!
//! We use [`AnimationTransitions`] internally for bone animations to achieve smooth transitions,
//! but there is an issue where interpolation fails if the initial poses of the source and target VRMAs differ.
//! If anyone has a good solution, please feel free to open an issue or submit a PR.
//!
//! 1. `VRMA_01.vrma`
//! 2. `VRMA_02.vrma`
//! 3. `VRMA_03.vrma`
//! 4. `different_pose.vrma`
//!
//! L: toggle the skirt limb pack
//!
//! Key 2 plays `VRMA_02`, whose bent knees make the skirt sink through the legs:
//! `AliciaSolid` has thigh colliders but nothing below mid-thigh.
//! Press L to add a full limb pack (upper legs, shins, upper arms, forearms)
//! with bone-local capsules measured from the model's bone geometry. This is a
//! mitigation, not a resolution: VRM point collision cannot fully resolve
//! deep-pose skirt-vs-limb overlap, and `different_pose` still leaves arm
//! overlap. Toggle before replaying pose 2 for the strongest effect; a held
//! deep pose already has strands inside the limbs.

use bevy::animation::RepeatAnimation;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;
use std::time::Duration;

#[derive(Component, Default)]
struct VrmaNo<const I: usize>;

#[derive(Component)]
struct HelpText;

#[derive(Resource, Default)]
struct LimbColliders {
    enabled: bool,
    spawned: Vec<Entity>,
    original_lengths: Vec<(Entity, usize)>,
}

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmaPlugin))
        .init_resource::<LimbColliders>()
        .add_systems(Startup, (spawn_directional_light, spawn_camera, spawn_vrm))
        .add_systems(
            Update,
            (
                play_vrma::<1>.run_if(input_just_pressed(KeyCode::Digit1)),
                play_vrma::<2>.run_if(input_just_pressed(KeyCode::Digit2)),
                play_vrma::<3>.run_if(input_just_pressed(KeyCode::Digit3)),
                play_vrma::<4>.run_if(input_just_pressed(KeyCode::Digit4)),
                toggle_limb_colliders.run_if(input_just_pressed(KeyCode::KeyL)),
            ),
        )
        .run();
}

fn spawn_directional_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 3.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera3d::default(), Transform::from_xyz(0., 0.8, 2.5)));
}

fn spawn_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.spawn((
        Text::new(help_text(false)),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(10.0),
            ..default()
        },
        HelpText,
    ));
    commands
        .spawn(VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")))
        .with_children(|cmd| {
            cmd.spawn((
                VrmaNo::<1>,
                VrmaHandle(asset_server.load("vrma/VRMA_01.vrma")),
            ));
            cmd.spawn((
                VrmaNo::<2>,
                VrmaHandle(asset_server.load("vrma/VRMA_02.vrma")),
            ));
            cmd.spawn((
                VrmaNo::<3>,
                VrmaHandle(asset_server.load("vrma/VRMA_03.vrma")),
            ));
            cmd.spawn((
                VrmaNo::<4>,
                VrmaHandle(asset_server.load("vrma/different_pose.vrma")),
            ));
        });
}

fn play_vrma<const I: usize>(
    mut commands: Commands,
    vrmas: Query<Entity, With<VrmaNo<I>>>,
) {
    let Ok(vrma_entity) = vrmas.single() else {
        return;
    };
    commands.trigger(PlayVrma {
        repeat: RepeatAnimation::Forever,
        transition_duration: Duration::from_millis(300),
        vrma: vrma_entity,
        reset_spring_bones: true,
    });
}

fn help_text(enabled: bool) -> String {
    let state = if enabled {
        "leg/arm capsules ON - replay 2: strands pushed out during the bend (held deep poses still touch)"
    } else {
        "leg/arm capsules OFF - press 2: skirt sinks through bent knees"
    };
    format!("1: VRMA_01   2: VRMA_02   3: VRMA_03   4: different_pose\nL: {state}")
}

fn toggle_limb_colliders(
    mut commands: Commands,
    mut state: ResMut<LimbColliders>,
    mut roots: Query<(Entity, &mut SpringRoot, &Name)>,
    searcher: ChildSearcher,
    parents: ParentSearcher,
    mut overlay: Query<&mut Text, With<HelpText>>,
) {
    if state.enabled {
        for collider in state.spawned.drain(..) {
            commands.entity(collider).despawn();
        }
        for (root, original_len) in state.original_lengths.drain(..) {
            if let Ok((_, mut spring_root, _)) = roots.get_mut(root) {
                spring_root.colliders.0.truncate(original_len);
            }
        }
        state.enabled = false;
    } else {
        let Some(vrm) = roots
            .iter()
            .find(|(_, _, name)| name.as_str().starts_with("skirt_"))
            .and_then(|(entity, _, _)| parents.find_vrm(entity))
        else {
            return;
        };
        let limb_bones = [
            (
                "LeftUpLeg",
                [-0.0007, -0.0363, 0.0001],
                [-0.0071, -0.3815, 0.0014],
                0.07,
            ),
            (
                "RightUpLeg",
                [0.0007, -0.0363, 0.0001],
                [0.0071, -0.3815, 0.0014],
                0.07,
            ),
            (
                "LeftLeg",
                [-0.0006, -0.0210, -0.0010],
                [-0.0132, -0.4405, -0.0212],
                0.050,
            ),
            (
                "RightLeg",
                [0.0006, -0.0210, -0.0010],
                [0.0132, -0.4405, -0.0212],
                0.050,
            ),
            (
                "LeftArm",
                [0.0104, -0.0003, 0.0000],
                [0.2194, -0.0060, 0.0008],
                0.045,
            ),
            (
                "RightArm",
                [-0.0104, -0.0003, 0.0000],
                [-0.2194, -0.0070, 0.0010],
                0.045,
            ),
            (
                "LeftForeArm",
                [0.0109, 0.0000, 0.0001],
                [0.2286, -0.0010, 0.0012],
                0.035,
            ),
            (
                "RightForeArm",
                [-0.0109, -0.0001, 0.0001],
                [-0.2286, -0.0011, 0.0014],
                0.035,
            ),
        ];
        let mut capsules = Vec::with_capacity(limb_bones.len());
        for (name, offset, tail, radius) in limb_bones {
            let Some(bone) = searcher.find_from_name(vrm, name) else {
                return;
            };
            capsules.push((
                bone,
                ColliderShape::Capsule(Capsule {
                    offset,
                    radius,
                    tail,
                }),
            ));
        }

        for (root_entity, mut spring_root, name) in roots.iter_mut() {
            if !name.as_str().starts_with("skirt_") {
                continue;
            }
            let original_len = spring_root.colliders.0.len();
            for (bone, shape) in &capsules {
                let collider = commands
                    .spawn((Transform::IDENTITY, ChildOf(*bone), *shape))
                    .id();
                spring_root.colliders.0.push((collider, *shape));
                state.spawned.push(collider);
            }
            state.original_lengths.push((root_entity, original_len));
        }
        state.enabled = true;
    }

    for mut text in overlay.iter_mut() {
        **text = help_text(state.enabled);
    }
}
