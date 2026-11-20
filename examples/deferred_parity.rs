use bevy::app::{App, PluginGroupBuilder, PostUpdate};
use bevy::asset::AssetServer;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::WgpuSettings;
use bevy::time::TimeUpdateStrategy;
use bevy::transform::prelude::TransformHelper;
use bevy::transform::systems::propagate_transforms_for;
use bevy_vrm1::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

const MODEL: &str = "vrm/AliciaSolid.vrm";
const TOL: f32 = 1e-4;
const FRAMES: usize = 300;
const CHECKPOINTS: [usize; 3] = [50, 150, 299];

#[derive(Component)]
struct DeferredStale;

#[derive(Resource, Default, Clone, Copy)]
struct Probe {
    armed: bool,
    before_error: f32,
    after_error: f32,
    samples: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arm {
    Default,
    Optout,
    Deferred,
}

impl Arm {
    fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Optout => "optout",
            Self::Deferred => "deferred",
        }
    }
}

#[derive(Clone)]
struct Pose {
    parent: Option<String>,
    transform: Transform,
    global: GlobalTransform,
}

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
        let name = world.get::<Name>(entity).map_or("-", |name| name.as_str());
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

fn probe_stale(
    probe: &mut Probe,
    stale: &Query<Entity, With<DeferredStale>>,
    globals: &Query<&GlobalTransform>,
    helper: &TransformHelper,
) {
    if !probe.armed {
        return;
    }
    for entity in stale {
        let (Ok(expected), Ok(stored)) =
            (helper.compute_global_transform(entity), globals.get(entity))
        else {
            continue;
        };
        probe.before_error = probe.before_error.max(global_diff(stored, &expected));
        probe.samples += 1;
    }
}

fn probe_before(
    mut probe: ResMut<Probe>,
    stale: Query<Entity, With<DeferredStale>>,
    globals: Query<&GlobalTransform>,
    helper: TransformHelper,
) {
    probe_stale(&mut probe, &stale, &globals, &helper);
}

fn probe_after(
    mut probe: ResMut<Probe>,
    stale: Query<Entity, With<DeferredStale>>,
    globals: Query<&GlobalTransform>,
    helper: TransformHelper,
) {
    if !probe.armed {
        return;
    }
    for entity in &stale {
        let (Ok(expected), Ok(stored)) =
            (helper.compute_global_transform(entity), globals.get(entity))
        else {
            continue;
        };
        probe.after_error = probe.after_error.max(global_diff(stored, &expected));
    }
}

struct Scene {
    app: App,
    arm: Arm,
    owner: Entity,
    dirty: Vec<Entity>,
    keys: BTreeMap<Entity, String>,
    joint_keys: BTreeSet<String>,
    stale_keys: BTreeSet<String>,
    poses: BTreeMap<usize, BTreeMap<String, Pose>>,
    probes: BTreeMap<usize, (f32, f32, usize)>,
}

fn build_scene(arm: Arm) -> Scene {
    let mut app = App::new();
    app.add_plugins(headless_default_plugins());
    app.add_plugins(VrmPlugin);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        16,
    )));
    if arm != Arm::Default {
        app.insert_resource(SkipPostSpringPropagation);
    }
    app.init_resource::<Probe>();
    if arm == Arm::Deferred {
        let deferred = propagate_transforms_for::<With<DeferredStale>>;
        app.add_systems(
            PostUpdate,
            probe_before
                .before(deferred)
                .before(VrmSystemSets::Constraints),
        );
        app.add_systems(PostUpdate, deferred.before(VrmSystemSets::Constraints));
        app.add_systems(
            PostUpdate,
            probe_after
                .after(deferred)
                .before(VrmSystemSets::Constraints),
        );
    }

    let model_handle = app.world().resource::<AssetServer>().load::<Gltf>(MODEL);
    let owner = app
        .world_mut()
        .spawn((VrmHandle(model_handle.clone()), Transform::default()))
        .id();
    app.finish();
    app.cleanup();

    let mut ready = false;
    for frame in 0..1200 {
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
        let loaded = app
            .world()
            .resource::<AssetServer>()
            .is_loaded(&model_handle);
        if initialized && loaded {
            ready = true;
            break;
        }
    }
    assert!(ready, "VRM assets did not initialize for {MODEL}");
    for _ in 0..10 {
        app.update();
    }

    let (simulated, stale) = {
        let mut query = app.world_mut().query_filtered::<Entity, With<SpringRoot>>();
        let roots = query.iter(app.world()).collect::<Vec<_>>();
        let mut simulated = BTreeSet::new();
        for root in roots {
            let members = app
                .world()
                .get::<SpringRoot>(root)
                .unwrap()
                .joints
                .0
                .clone();
            if members.len() >= 2 {
                simulated.extend(members[..members.len() - 1].iter().copied());
            }
        }
        let mut stale = Vec::new();
        if arm == Arm::Deferred {
            for joint in &simulated {
                let mut stack = app
                    .world()
                    .get::<Children>(*joint)
                    .map_or_else(Vec::new, |children| children.iter().collect());
                while let Some(entity) = stack.pop() {
                    if simulated.contains(&entity) {
                        continue;
                    }
                    if app.world().get::<Transform>(entity).is_some() {
                        stale.push(entity);
                        if let Some(children) = app.world().get::<Children>(entity) {
                            stack.extend(children.iter());
                        }
                    }
                }
            }
            stale.sort_unstable();
            stale.dedup();
            for entity in &stale {
                app.world_mut().entity_mut(*entity).insert(DeferredStale);
            }
        }
        (simulated, stale)
    };

    let mut dirty = Vec::new();
    let mut query = app.world_mut().query_filtered::<Entity, With<SpringRoot>>();
    for root in query.iter(app.world()) {
        let first = app
            .world()
            .get::<SpringRoot>(root)
            .and_then(|spring| spring.joints.0.first().copied());
        let Some(first) = first else {
            continue;
        };
        if let Some(parent) = app.world().get::<ChildOf>(first).map(ChildOf::parent)
            && !simulated.contains(&parent)
            && !dirty.contains(&parent)
        {
            dirty.push(parent);
        }
    }
    if dirty.is_empty() {
        dirty.push(owner);
    }

    let mut keys = BTreeMap::new();
    let initial = collect_poses(&app, owner, &mut keys);
    let key_set = |entities: &BTreeSet<Entity>| {
        entities
            .iter()
            .filter_map(|entity| keys.get(entity).cloned())
            .collect::<BTreeSet<_>>()
    };
    let joint_keys = key_set(&simulated);
    let stale_keys = initial
        .keys()
        .filter(|key| {
            if joint_keys.contains(*key) {
                return false;
            }
            let mut cursor = initial.get(*key).and_then(|pose| pose.parent.as_deref());
            while let Some(parent) = cursor {
                if joint_keys.contains(parent) {
                    return true;
                }
                cursor = initial.get(parent).and_then(|pose| pose.parent.as_deref());
            }
            false
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(!joint_keys.is_empty(), "no simulated spring joints found");
    assert!(
        arm != Arm::Deferred || stale.len() == stale_keys.len(),
        "stale marker count does not match the classified stale set"
    );

    Scene {
        app,
        arm,
        owner,
        dirty,
        keys,
        joint_keys,
        stale_keys,
        poses: BTreeMap::new(),
        probes: BTreeMap::new(),
    }
}

fn drive_frame(
    scene: &mut Scene,
    frame: usize,
) {
    let rotation = Quat::from_rotation_y(frame as f32 * 0.017);
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
}

fn run_frames(scene: &mut Scene) {
    let needed = CHECKPOINTS
        .iter()
        .flat_map(|frame| [frame.saturating_sub(1), *frame])
        .chain(std::iter::once(0))
        .collect::<BTreeSet<_>>();
    for frame in 0..FRAMES {
        drive_frame(scene, frame);
        if needed.contains(&frame) {
            scene.app.world_mut().resource_mut::<Probe>().armed = true;
        }
        scene.app.update();
        if needed.contains(&frame) {
            let probe = *scene.app.world().resource::<Probe>();
            scene.probes.insert(
                frame,
                (probe.before_error, probe.after_error, probe.samples),
            );
            scene.app.world_mut().resource_mut::<Probe>().armed = false;
            scene.poses.insert(
                frame,
                collect_poses(&scene.app, scene.owner, &mut scene.keys),
            );
        }
    }
}

fn transform_diff(
    a: &Transform,
    b: &Transform,
) -> f32 {
    let mut max = 0.0f32;
    for (x, y) in a
        .translation
        .to_array()
        .iter()
        .zip(b.translation.to_array())
    {
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

fn global_diff(
    a: &GlobalTransform,
    b: &GlobalTransform,
) -> f32 {
    a.to_matrix()
        .to_cols_array()
        .iter()
        .zip(b.to_matrix().to_cols_array())
        .fold(0.0f32, |max, (x, y)| max.max((x - y).abs()))
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

fn compare(
    reference: &Scene,
    candidate: &Scene,
) -> bool {
    let mut failures = Vec::new();
    let label = candidate.arm.label();
    if reference.joint_keys != candidate.joint_keys || reference.stale_keys != candidate.stale_keys
    {
        failures.push("entity classification differs between arms".to_string());
    }

    for &frame in &CHECKPOINTS {
        let Some(reference_poses) = reference.poses.get(&frame) else {
            failures.push(format!("frame {frame}: missing reference snapshot"));
            continue;
        };
        let Some(candidate_poses) = candidate.poses.get(&frame) else {
            failures.push(format!("frame {frame}: missing candidate snapshot"));
            continue;
        };
        let Some(previous) = candidate.poses.get(&(frame - 1)) else {
            failures.push(format!(
                "frame {frame}: missing previous candidate snapshot"
            ));
            continue;
        };

        let mut max_joint_global = 0.0f32;
        let mut max_stable_global = 0.0f32;
        let mut max_stale_global = 0.0f32;
        let mut max_stale_relation = 0.0f32;
        let mut reference_fresh = 0.0f32;
        let mut joint_motion = 0.0f32;

        for (key, reference_pose) in reference_poses {
            let Some(candidate_pose) = candidate_poses.get(key) else {
                failures.push(format!("frame {frame} {key}: entity missing in candidate"));
                continue;
            };
            let local = transform_diff(&reference_pose.transform, &candidate_pose.transform);
            if local > TOL {
                failures.push(format!(
                    "frame {frame} {key} local Transform deviation {local:.3e}"
                ));
            }
            let global = global_diff(&reference_pose.global, &candidate_pose.global);
            if reference.joint_keys.contains(key) {
                max_joint_global = max_joint_global.max(global);
                if global > TOL {
                    failures.push(format!(
                        "frame {frame} {key} spring joint GlobalTransform deviation {global:.3e}"
                    ));
                }
            } else if reference.stale_keys.contains(key) {
                max_stale_global = max_stale_global.max(global);
                let Some(expected) =
                    propagated_global(key, candidate_poses, previous, &candidate.joint_keys)
                else {
                    failures.push(format!("frame {frame} {key} stale entity has no path"));
                    continue;
                };
                let relation = global_diff(&candidate_pose.global, &expected);
                max_stale_relation = max_stale_relation.max(relation);
                if relation > TOL {
                    failures.push(format!(
                        "frame {frame} {key} stale descendant deviates from one-frame propagation by {relation:.3e}"
                    ));
                }
                let reference_expected =
                    propagated_global(key, reference_poses, reference_poses, &reference.joint_keys);
                let fresh = reference_expected.map_or(f32::INFINITY, |expected| {
                    global_diff(&reference_pose.global, &expected)
                });
                reference_fresh = reference_fresh.max(fresh);
                if fresh > TOL {
                    failures.push(format!(
                        "frame {frame} {key} reference arm leaves stale descendant unpublished by {fresh:.3e}"
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

        for key in &reference.joint_keys {
            let Some(current) = reference_poses.get(key) else {
                continue;
            };
            let Some(before) = reference.poses.get(&0).and_then(|poses| poses.get(key)) else {
                continue;
            };
            joint_motion = joint_motion.max(transform_diff(&current.transform, &before.transform));
        }
        if joint_motion <= TOL {
            failures.push(format!(
                "frame {frame} spring joints did not move since frame 0 (motion {joint_motion:.3e})"
            ));
        }

        println!(
            "checkpoint arm={label} frame={frame} stale={} joint_global_err={:.3e} stable_global_err={:.3e} stale_global_err={:.3e} stale_relation_err={:.3e} reference_fresh_err={:.3e} motion={:.3e}",
            reference.stale_keys.len(),
            max_joint_global,
            max_stable_global,
            max_stale_global,
            max_stale_relation,
            reference_fresh,
            joint_motion,
        );
    }

    if candidate.arm == Arm::Deferred {
        for &frame in &CHECKPOINTS {
            let Some((before_error, after_error, samples)) = candidate.probes.get(&frame) else {
                failures.push(format!("frame {frame}: missing deferred probe snapshot"));
                continue;
            };
            if *samples == 0 {
                failures.push(format!(
                    "frame {frame}: deferred probe saw no stale entities"
                ));
            }
            if *after_error > TOL {
                failures.push(format!(
                    "frame {frame}: deferred pass leaves stale descendants unpublished by {after_error:.3e}"
                ));
            }
            println!(
                "probe arm={label} frame={frame} before_error={before_error:.3e} after_error={after_error:.3e} samples={samples}"
            );
        }
    }

    if failures.is_empty() {
        println!("result arm={label} PASS");
        true
    } else {
        println!("result arm={label} FAIL ({} assertions)", failures.len());
        for failure in &failures {
            println!("  {label} {failure}");
        }
        false
    }
}

fn main() {
    let mut reference = build_scene(Arm::Default);
    let mut optout = build_scene(Arm::Optout);
    let mut deferred = build_scene(Arm::Deferred);
    run_frames(&mut reference);
    run_frames(&mut optout);
    run_frames(&mut deferred);

    println!("model={MODEL} frames={FRAMES} checkpoints={CHECKPOINTS:?} tol={TOL}");
    let mut failures = Vec::new();
    if !compare(&reference, &optout) {
        failures.push("optout");
    }
    if !compare(&reference, &deferred) {
        failures.push("deferred");
    }
    if failures.is_empty() {
        println!(
            "verdict PASS: deferred prototype keeps spring parity and publishes stale leaves at the head of PostUpdate"
        );
    } else {
        println!("verdict FAIL: {}", failures.join(", "));
        std::process::exit(1);
    }
}
