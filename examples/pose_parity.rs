use bevy::animation::RepeatAnimation;
use bevy::app::{App, PluginGroupBuilder};
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::WgpuSettings;
use bevy::time::TimeUpdateStrategy;
use bevy_vrm1::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

const TOL: f32 = 1e-4;
const FRAMES: usize = 300;
const CHECKPOINTS: [usize; 3] = [50, 150, 299];
const MODELS: [(&str, &str); 2] = [
    ("AliciaSolid", "vrm/AliciaSolid.vrm"),
    ("Elmer", "vrm/Elmer.vrm"),
];

fn headless_default_plugins() -> PluginGroupBuilder {
    DefaultPlugins
        .build()
        .disable::<bevy::winit::WinitPlugin>()
        .disable::<bevy::log::LogPlugin>()
        .set(RenderPlugin {
            render_creation: WgpuSettings {
                backends: None,
                ..default()
            }
            .into(),
            ..default()
        })
}

#[derive(Resource, Default)]
struct LoadedVrmas(Vec<Entity>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drive {
    Synthetic,
    Vrma(&'static str),
}

impl Drive {
    fn label(self) -> String {
        match self {
            Self::Synthetic => "synthetic".to_string(),
            Self::Vrma(path) => format!("vrma:{path}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Inventory {
    spring_roots: usize,
    declared_joints: usize,
    simulated_joints: usize,
    leaf_tails: usize,
    colliders: usize,
    entities: usize,
    vrma_loaded: bool,
}

#[derive(Clone)]
struct Pose {
    parent: Option<String>,
    transform: Transform,
    global: GlobalTransform,
}

fn collect_poses(
    app: &App,
    owner: Entity,
    keys: &mut BTreeMap<Entity, String>,
) -> BTreeMap<String, Pose> {
    let world = app.world();
    let mut poses = BTreeMap::new();

    fn visit(
        world: &World,
        entity: Entity,
        parent_key: &str,
        index: usize,
        keys: &mut BTreeMap<Entity, String>,
        poses: &mut BTreeMap<String, Pose>,
    ) {
        let (Some(transform), Some(global)) = (
            world.get::<Transform>(entity),
            world.get::<GlobalTransform>(entity),
        ) else {
            return;
        };
        let name = world
            .get::<Name>(entity)
            .map_or("-", |name| name.as_str());
        let key = format!("{parent_key}/{name}#{index}");
        keys.insert(entity, key.clone());
        poses.insert(
            key.clone(),
            Pose {
                parent: (parent_key != "vrm").then(|| parent_key.to_string()),
                transform: *transform,
                global: *global,
            },
        );
        if let Some(children) = world.get::<Children>(entity) {
            for (index, child) in children.iter().enumerate() {
                visit(world, child, &key, index, keys, poses);
            }
        }
    }

    visit(world, owner, "vrm", 0, keys, &mut poses);
    poses
}

struct Scene {
    app: App,
    owner: Entity,
    manual_drive: bool,
    dirty: Vec<Entity>,
    look_target: Option<Entity>,
    joint_keys: BTreeSet<String>,
    tail_keys: BTreeSet<String>,
    leaf_keys: BTreeSet<String>,
    stale_keys: BTreeSet<String>,
    inventory: Inventory,
    poses: BTreeMap<usize, BTreeMap<String, Pose>>,
}

fn build_scene(
    model: &'static str,
    drive: Drive,
    skip_post_spring: bool,
) -> Scene {
    let mut app = App::new();
    app.add_plugins(headless_default_plugins());
    if matches!(drive, Drive::Vrma(_)) {
        app.add_plugins(VrmaPlugin);
    }
    app.add_plugins(VrmPlugin);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(16)));
    if skip_post_spring {
        app.insert_resource(SkipPostSpringPropagation);
    }
    app.init_resource::<LoadedVrmas>()
        .add_observer(|event: On<LoadedVrma>, mut loaded: ResMut<LoadedVrmas>| {
            loaded.0.push(event.vrma);
        });

    let model_handle = app
        .world()
        .resource::<AssetServer>()
        .load::<Gltf>(model);
    let owner = app
        .world_mut()
        .spawn((VrmHandle(model_handle.clone()), Transform::default()))
        .id();

    let vrma_handle = match drive {
        Drive::Synthetic => None,
        Drive::Vrma(path) => Some(
            app.world()
                .resource::<AssetServer>()
                .load::<VrmaAsset>(path),
        ),
    };
    let vrma = vrma_handle.as_ref().map(|handle| {
        app.world_mut()
            .spawn((ChildOf(owner), VrmaHandle(handle.clone())))
            .id()
    });

    app.finish();
    app.cleanup();

    let mut ready = false;
    for frame in 0..2000 {
        app.update();
        if frame % 16 == 0 {
            std::thread::yield_now();
        }
        let initialized = app
            .world_mut()
            .query_filtered::<Entity, With<Initialized>>()
            .iter(app.world())
            .count()
            >= 1;
        let model_loaded = app
            .world()
            .resource::<AssetServer>()
            .is_loaded(&model_handle);
        let vrma_ready = match (&vrma_handle, vrma) {
            (Some(handle), Some(entity)) => {
                app.world().resource::<AssetServer>().is_loaded(handle)
                    && app
                        .world()
                        .resource::<LoadedVrmas>()
                        .0
                        .contains(&entity)
            }
            _ => true,
        };
        if initialized && model_loaded && vrma_ready {
            ready = true;
            break;
        }
    }
    assert!(ready, "assets did not initialize for {model} {drive:?}");

    for _ in 0..10 {
        app.update();
    }

    if let Some(vrma) = vrma {
        app.world_mut().trigger(PlayVrma {
            vrma,
            repeat: RepeatAnimation::Forever,
            transition_duration: Duration::ZERO,
            reset_spring_bones: false,
        });
    }

    let mut dirty = Vec::new();
    let mut look_target = None;
    if matches!(drive, Drive::Synthetic) {
        let spring_joints: BTreeSet<Entity> = {
            let mut query = app.world_mut().query_filtered::<Entity, With<SpringRoot>>();
            query
                .iter(app.world())
                .flat_map(|root| {
                    app.world()
                        .get::<SpringRoot>(root)
                        .unwrap()
                        .joints
                        .0
                        .iter()
                        .copied()
                })
                .collect()
        };
        let mut roots = app.world_mut().query_filtered::<Entity, With<SpringRoot>>();
        for root in roots.iter(app.world()) {
            let first = app
                .world()
                .get::<SpringRoot>(root)
                .and_then(|spring| spring.joints.0.first().copied());
            let Some(first) = first else {
                continue;
            };
            if let Some(parent) = app.world().get::<ChildOf>(first).map(|child| child.parent())
                && !spring_joints.contains(&parent)
                && !dirty.contains(&parent)
            {
                dirty.push(parent);
            }
        }
        if dirty.is_empty() {
            dirty.push(owner);
        }
        look_target = Some(app.world_mut().spawn(Transform::default()).id());
        app.world_mut()
            .entity_mut(owner)
            .insert(LookAt::Target(look_target.unwrap()));
    }

    for frame in 0..30 {
        if matches!(drive, Drive::Synthetic) {
            let angle = frame as f32 * 0.017;
            let rotation = Quat::from_rotation_y(angle);
            for entity in &dirty {
                if let Some(mut transform) = app.world_mut().entity_mut(*entity).get_mut::<Transform>() {
                    transform.rotation = rotation;
                }
            }
            if let Some(target) = look_target
                && let Some(mut transform) = app.world_mut().entity_mut(target).get_mut::<Transform>()
            {
                transform.translation = Vec3::new(
                    0.4 * (frame as f32 * 0.05).sin(),
                    1.4,
                    0.8 * (frame as f32 * 0.05).cos(),
                );
            }
        }
        app.update();
    }

    let mut keys = BTreeMap::new();
    let initial = collect_poses(&app, owner, &mut keys);
    let spring_info = {
        let mut query = app.world_mut().query_filtered::<Entity, With<SpringRoot>>();
        let roots = query.iter(app.world()).collect::<Vec<_>>();
        let mut declared = BTreeSet::new();
        let mut simulated = BTreeSet::new();
        let mut simulated_tails = BTreeSet::new();
        let mut leaf_tails = BTreeSet::new();
        let mut colliders = 0;
        for root in &roots {
            let spring = app.world().get::<SpringRoot>(*root).unwrap();
            let members = spring.joints.0.iter().copied().collect::<Vec<_>>();
            declared.extend(members.iter().copied());
            if members.len() >= 2 {
                simulated.extend(members[..members.len() - 1].iter().copied());
                simulated_tails.insert(members[members.len() - 2]);
                leaf_tails.insert(members[members.len() - 1]);
            }
            colliders += spring.colliders.0.len();
        }
        (roots.len(), declared, simulated, simulated_tails, leaf_tails, colliders)
    };
    let (spring_roots, declared_joints, simulated_joints, simulated_tails, leaf_tails, colliders) =
        spring_info;
    let key_set = |entities: &BTreeSet<Entity>| {
        entities
            .iter()
            .filter_map(|entity| keys.get(entity).cloned())
            .collect::<BTreeSet<_>>()
    };
    let joint_keys = key_set(&simulated_joints);
    let tail_keys = key_set(&simulated_tails);
    let leaf_keys = key_set(&leaf_tails);
    let stale_keys = initial
        .iter()
        .filter(|(key, pose)| {
            if joint_keys.contains(*key) {
                return false;
            }
            let mut parent = pose.parent.as_deref();
            while let Some(parent_key) = parent {
                if joint_keys.contains(parent_key) {
                    return true;
                }
                parent = initial.get(parent_key).and_then(|pose| pose.parent.as_deref());
            }
            false
        })
        .map(|(key, _)| key.clone())
        .collect::<BTreeSet<_>>();
    let inventory = Inventory {
        spring_roots,
        declared_joints: declared_joints.len(),
        simulated_joints: joint_keys.len(),
        leaf_tails: leaf_keys.len(),
        colliders,
        entities: initial.len(),
        vrma_loaded: vrma.is_some(),
    };

    Scene {
        app,
        owner,
        manual_drive: matches!(drive, Drive::Synthetic),
        dirty,
        look_target,
        joint_keys,
        tail_keys,
        leaf_keys,
        stale_keys,
        inventory,
        poses: BTreeMap::new(),
    }
}

fn drive_frame(scene: &mut Scene, frame: usize) {
    if !scene.manual_drive {
        return;
    }
    let angle = frame as f32 * 0.017;
    let rotation = Quat::from_rotation_y(angle);
    for entity in &scene.dirty {
        if let Some(mut transform) = scene
            .app
            .world_mut()
            .entity_mut(*entity)
            .get_mut::<Transform>()
        {
            transform.rotation = rotation;
        }
    }
    if let Some(target) = scene.look_target
        && let Some(mut transform) = scene
            .app
            .world_mut()
            .entity_mut(target)
            .get_mut::<Transform>()
    {
        transform.translation = Vec3::new(
            0.4 * (frame as f32 * 0.05).sin(),
            1.4,
            0.8 * (frame as f32 * 0.05).cos(),
        );
    }
}

fn run_frames(scene: &mut Scene) {
    let needed = CHECKPOINTS
        .iter()
        .flat_map(|frame| [frame.saturating_sub(1), *frame])
        .chain(std::iter::once(0))
        .collect::<BTreeSet<_>>();
    let mut keys = BTreeMap::new();
    for frame in 0..FRAMES {
        drive_frame(scene, frame);
        scene.app.update();
        if needed.contains(&frame) {
            scene
                .poses
                .insert(frame, collect_poses(&scene.app, scene.owner, &mut keys));
        }
    }
}

fn transform_diff(a: &Transform, b: &Transform) -> f32 {
    let mut max = 0.0f32;
    for (x, y) in a.translation.to_array().iter().zip(b.translation.to_array()) {
        max = max.max((x - y).abs());
    }
    for (x, y) in a.scale.to_array().iter().zip(b.scale.to_array()) {
        max = max.max((x - y).abs());
    }
    for (x, y) in a.rotation.to_array().iter().zip(b.rotation.to_array()) {
        max = max.max((x - y).abs());
    }
    max
}

fn propagated_global(
    key: &str,
    current: &BTreeMap<String, Pose>,
    previous: &BTreeMap<String, Pose>,
    joint_keys: &BTreeSet<String>,
) -> Option<GlobalTransform> {
    let mut chain = Vec::new();
    let mut at = key;
    loop {
        let pose = current.get(at)?;
        chain.push(at);
        let Some(parent) = pose.parent.as_deref() else {
            break;
        };
        at = parent;
    }
    chain.reverse();
    let mut global = current.get(*chain.first()?)?.global;
    for at in chain.iter().skip(1) {
        let transform = if joint_keys.contains(*at) {
            previous
                .get(*at)
                .map_or(current.get(*at)?.transform, |pose| pose.transform)
        } else {
            current.get(*at)?.transform
        };
        global = global.mul_transform(transform);
    }
    Some(global)
}

fn global_diff(a: &GlobalTransform, b: &GlobalTransform) -> f32 {
    a.to_matrix()
        .to_cols_array()
        .iter()
        .zip(b.to_matrix().to_cols_array())
        .fold(0.0f32, |max, (x, y)| max.max((x - y).abs()))
}

fn compare(model: &str, drive: Drive, enabled: &Scene, disabled: &Scene) -> bool {
    let label = format!("{model}/{}", drive.label());
    let mut failures = Vec::new();
    println!(
        "model={model} drive={} inventory_enabled={:?} inventory_disabled={:?}",
        drive.label(),
        enabled.inventory,
        disabled.inventory
    );
    if enabled.inventory != disabled.inventory {
        failures.push("spring/entity inventory differs between arms".to_string());
    }
    if enabled.joint_keys != disabled.joint_keys
        || enabled.tail_keys != disabled.tail_keys
        || enabled.leaf_keys != disabled.leaf_keys
        || enabled.stale_keys != disabled.stale_keys
    {
        failures.push("entity classification differs between arms".to_string());
    }

    for &frame in &CHECKPOINTS {
        let Some(a) = enabled.poses.get(&frame) else {
            failures.push(format!("frame {frame}: missing enabled snapshot"));
            continue;
        };
        let Some(b) = disabled.poses.get(&frame) else {
            failures.push(format!("frame {frame}: missing disabled snapshot"));
            continue;
        };
        let Some(prev_disabled) = disabled.poses.get(&(frame - 1)) else {
            failures.push(format!("frame {frame}: missing previous disabled snapshot"));
            continue;
        };

        let mut max_local = 0.0f32;
        let mut max_joint_global = 0.0f32;
        let mut max_stable_global = 0.0f32;
        let mut max_stale_global = 0.0f32;
        let mut max_stale_relation = 0.0f32;
        let mut max_tail_self_publish = 0.0f32;
        let mut max_tail_arm_global = 0.0f32;
        let mut worst_tail: Option<(String, f32)> = None;
        let mut motion = 0.0f32;

        for (key, pose_a) in a {
            let Some(pose_b) = b.get(key) else {
                failures.push(format!("frame {frame} {key}: entity missing in disabled arm"));
                continue;
            };
            let local = transform_diff(&pose_a.transform, &pose_b.transform);
            max_local = max_local.max(local);
            if local > TOL {
                failures.push(format!(
                    "frame {frame} {key} local Transform deviation {local:.3e}"
                ));
            }

            let is_joint = enabled.joint_keys.contains(key);
            let is_stale = enabled.stale_keys.contains(key);
            let global = global_diff(&pose_a.global, &pose_b.global);
            if is_joint {
                max_joint_global = max_joint_global.max(global);
                if global > TOL {
                    failures.push(format!(
                        "frame {frame} {key} spring joint GlobalTransform deviation {global:.3e}"
                    ));
                }
            } else if is_stale {
                max_stale_global = max_stale_global.max(global);
                let Some(expected_b) =
                    propagated_global(key, b, prev_disabled, &disabled.joint_keys)
                else {
                    failures.push(format!(
                        "frame {frame} {key} stale entity has no propagation path in disabled arm"
                    ));
                    continue;
                };
                let relation = global_diff(&pose_b.global, &expected_b);
                max_stale_relation = max_stale_relation.max(relation);
                if relation > TOL {
                    failures.push(format!(
                        "frame {frame} {key} stale descendant deviates from one-frame propagation by {relation:.3e}"
                    ));
                }
                let expected_a = propagated_global(key, a, a, &enabled.joint_keys);
                let fresh_a = expected_a.map_or(f32::INFINITY, |expected| {
                    global_diff(&pose_a.global, &expected)
                });
                if fresh_a > TOL {
                    failures.push(format!(
                        "frame {frame} {key} enabled arm leaves stale descendant unpublished by {fresh_a:.3e}"
                    ));
                }
            } else {
                max_stable_global = max_stable_global.max(global);
                if global > TOL {
                    failures.push(format!(
                        "frame {frame} {key} non-spring GlobalTransform deviation {global:.3e}"
                    ));
                }
            }
        }

        for key in &enabled.tail_keys {
            let Some(tail_a) = a.get(key) else {
                failures.push(format!("frame {frame} tail {key} missing in enabled arm"));
                continue;
            };
            let Some(tail_b) = b.get(key) else {
                failures.push(format!("frame {frame} tail {key} missing in disabled arm"));
                continue;
            };
            let arm_diff = global_diff(&tail_a.global, &tail_b.global);
            max_tail_arm_global = max_tail_arm_global.max(arm_diff);
            if worst_tail.as_ref().is_none_or(|(_, worst)| arm_diff > *worst) {
                worst_tail = Some((key.clone(), arm_diff));
            }
            for (arm, tail) in [(a, tail_a), (b, tail_b)] {
                let parent_key = tail.parent.as_deref().unwrap_or("");
                let Some(parent) = arm.get(parent_key) else {
                    failures.push(format!("frame {frame} tail {key} parent {parent_key} missing"));
                    continue;
                };
                let expected = parent.global.mul_transform(tail.transform);
                let error = global_diff(&tail.global, &expected);
                max_tail_self_publish = max_tail_self_publish.max(error);
                if error > TOL {
                    failures.push(format!(
                        "frame {frame} tail {key} self-published GlobalTransform error {error:.3e}"
                    ));
                }
            }
        }

        let baseline = enabled.poses.get(&0);
        for key in &enabled.joint_keys {
            let Some(current) = a.get(key) else {
                continue;
            };
            let Some(before) = baseline.and_then(|baseline| baseline.get(key)) else {
                continue;
            };
            motion = motion.max(transform_diff(&current.transform, &before.transform));
        }

        println!(
            "checkpoint frame={frame} entities={} simulated_joints={} stale={} stable={} simulated_tails={} leaf_tails={} max_local={:.3e} max_joint_gtf={:.3e} max_stable_gtf={:.3e} max_stale_gtf={:.3e} stale_relation_err={:.3e} tail_self_publish_err={:.3e} tail_arm_gtf={:.3e} joint_motion={:.3e}",
            a.len(),
            enabled.joint_keys.len(),
            enabled.stale_keys.len(),
            a.len() - enabled.joint_keys.len() - enabled.stale_keys.len(),
            enabled.tail_keys.len(),
            enabled.leaf_keys.len(),
            max_local,
            max_joint_global,
            max_stable_global,
            max_stale_global,
            max_stale_relation,
            max_tail_self_publish,
            max_tail_arm_global,
            motion
        );
        if let Some((tail_key, tail_diff)) = &worst_tail
            && *tail_diff > TOL
        {
            let tail_a = &a[tail_key];
            let tail_b = &b[tail_key];
            let parent_key = tail_a.parent.as_deref().unwrap_or("");
            let parent_a = &a[parent_key];
            let parent_b = &b[parent_key];
            println!(
                "  worst tail {tail_key} arm_diff={tail_diff:.3e} parent_gtf_diff={:.3e} parent_local_diff={:.3e} a_tail_gtf={:?} a_expected={:?} b_tail_gtf={:?} b_expected={:?}",
                global_diff(&parent_a.global, &parent_b.global),
                transform_diff(&parent_a.transform, &parent_b.transform),
                tail_a.global.to_matrix().to_cols_array(),
                parent_a.global.mul_transform(tail_a.transform).to_matrix().to_cols_array(),
                tail_b.global.to_matrix().to_cols_array(),
                parent_b.global.mul_transform(tail_b.transform).to_matrix().to_cols_array(),
            );
        }
        if motion <= TOL {
            failures.push(format!(
                "frame {frame} spring joints did not move since frame 0 (motion {motion:.3e})"
            ));
        }
    }

    if failures.is_empty() {
        println!("result {label} PASS");
        true
    } else {
        println!("result {label} FAIL ({} assertions)", failures.len());
        for failure in &failures {
            println!("  {label} {failure}");
        }
        false
    }
}

fn main() {
    let mut vrma_files = std::fs::read_dir("assets/vrma")
        .expect("assets/vrma")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "vrma"))
        .collect::<Vec<_>>();
    vrma_files.sort();
    println!(
        "vrma inventory: {}",
        vrma_files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let drives = [
        Drive::Synthetic,
        Drive::Vrma("vrma/VRMA_01.vrma"),
        Drive::Vrma("vrma/VRMA_02.vrma"),
        Drive::Vrma("vrma/VRMA_03.vrma"),
        Drive::Vrma("vrma/different_pose.vrma"),
    ];
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut failures = Vec::new();
    for (model_name, model_path) in MODELS {
        if args.first().is_some_and(|filter| !model_name.contains(filter)) {
            continue;
        }
        for drive in drives {
            if args
                .get(1)
                .is_some_and(|filter| !drive.label().contains(filter))
            {
                continue;
            }
            let mut enabled = build_scene(model_path, drive, false);
            let mut disabled = build_scene(model_path, drive, true);
            run_frames(&mut enabled);
            run_frames(&mut disabled);
            if !compare(model_name, drive, &enabled, &disabled) {
                failures.push(format!("{model_name}/{}", drive.label()));
            }
        }
    }
    if failures.is_empty() {
        println!("verdict PASS: the gate preserves pose correctness on both real models");
    } else {
        println!("verdict FAIL: {}", failures.join(", "));
        std::process::exit(1);
    }
}
