use bevy::app::{App, PluginGroupBuilder, PostUpdate};
use bevy::asset::AssetServer;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::WgpuSettings;
use bevy::time::TimeUpdateStrategy;
use bevy::transform::systems::propagate_transforms_for;
use bevy_vrm1::prelude::*;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const MODEL: &str = "vrm/AliciaSolid.vrm";

#[derive(Component)]
struct DeferredStale;

#[derive(Resource, Default, Clone, Copy)]
struct GateStats {
    ran: usize,
    skipped: usize,
}

fn spring_joint_transform_changed(
    joints: Query<(), (With<SpringJointProps>, Changed<Transform>)>,
) -> bool {
    !joints.is_empty()
}

fn count_gate(
    joints: Query<(), (With<SpringJointProps>, Changed<Transform>)>,
    mut stats: ResMut<GateStats>,
) {
    if joints.is_empty() {
        stats.skipped += 1;
    } else {
        stats.ran += 1;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arm {
    Default,
    Optout,
    Marker,
    Deferred,
    DeferredChanged,
}

impl Arm {
    fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Optout => "optout",
            Self::Marker => "marker",
            Self::Deferred => "deferred",
            Self::DeferredChanged => "deferred_changed",
        }
    }
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

struct Scene {
    app: App,
    dirty: Vec<Entity>,
    stale: Vec<Entity>,
    transform_count: usize,
    spring_root_count: usize,
    spring_joint_count: usize,
}

fn build_scene(arm: Arm, instances: usize) -> Scene {
    let mut app = App::new();
    app.add_plugins((headless_default_plugins(), VrmPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(16)));
    if arm != Arm::Default {
        app.insert_resource(SkipPostSpringPropagation);
    }
    if arm == Arm::Deferred {
        app.add_systems(
            PostUpdate,
            propagate_transforms_for::<With<DeferredStale>>.before(VrmSystemSets::Constraints),
        );
    }
    if arm == Arm::DeferredChanged {
        app.init_resource::<GateStats>();
        app.add_systems(
            PostUpdate,
            propagate_transforms_for::<With<DeferredStale>>
                .before(VrmSystemSets::Constraints)
                .run_if(spring_joint_transform_changed),
        );
        app.add_systems(
            PostUpdate,
            count_gate.before(VrmSystemSets::Constraints),
        );
    }

    let handles: Vec<Handle<Gltf>> = (0..instances)
        .map(|i| {
            let handle = app
                .world()
                .resource::<AssetServer>()
                .load::<Gltf>(MODEL);
            app.world_mut().spawn((
                VrmHandle(handle.clone()),
                Transform::from_xyz(i as f32 * 2.0, 0.0, 0.0),
            ));
            handle
        })
        .collect();
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
            .count();
        let loaded = handles
            .iter()
            .all(|handle| app.world().resource::<AssetServer>().is_loaded(handle));
        if initialized >= instances && loaded {
            ready = true;
            break;
        }
    }
    assert!(
        ready,
        "VRM assets did not initialize: expected {instances} avatars from {MODEL}"
    );
    for _ in 0..10 {
        app.update();
    }

    let (spring_roots, spring_joints, simulated) = {
        let mut query = app.world_mut().query_filtered::<Entity, With<SpringRoot>>();
        let roots = query.iter(app.world()).collect::<Vec<_>>();
        let mut joints = Vec::new();
        let mut simulated = BTreeSet::new();
        for root in &roots {
            let members = app
                .world()
                .get::<SpringRoot>(*root)
                .unwrap()
                .joints
                .0
                .clone();
            joints.extend(members.iter().copied());
            if members.len() >= 2 {
                simulated.extend(members[..members.len() - 1].iter().copied());
            }
        }
        (roots, joints, simulated)
    };
    assert!(!spring_roots.is_empty(), "the model has no spring roots");

    let mut dirty = spring_roots
        .iter()
        .filter_map(|root| {
            let first = app.world().get::<SpringRoot>(*root)?.joints.0.first().copied()?;
            let parent = app.world().get::<ChildOf>(first)?.parent();
            (!simulated.contains(&parent)).then_some(parent)
        })
        .collect::<Vec<_>>();
    if dirty.is_empty() {
        dirty = app
            .world_mut()
            .query_filtered::<Entity, With<VrmHandle>>()
            .iter(app.world())
            .collect();
    }
    assert!(!dirty.is_empty(), "the scene has no transform to dirty");

    let stale = if arm != Arm::Default && arm != Arm::Optout {
        let mut stale = Vec::new();
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
        stale
    } else {
        Vec::new()
    };

    let transform_count = app
        .world_mut()
        .query_filtered::<Entity, With<Transform>>()
        .iter(app.world())
        .count();

    Scene {
        app,
        dirty,
        stale,
        transform_count,
        spring_root_count: spring_roots.len(),
        spring_joint_count: spring_joints.len(),
    }
}

fn dirty_transforms(scene: &mut Scene, frame: usize) {
    let angle = (frame as f32) * 0.017;
    let rotation = Quat::from_rotation_y(angle);
    for entity in &scene.dirty {
        scene
            .app
            .world_mut()
            .entity_mut(*entity)
            .get_mut::<Transform>()
            .expect("the dirty target keeps its Transform")
            .rotation = rotation;
    }
}

fn timed_frames(scene: &mut Scene, warmup: usize, frames: usize, dirty: bool) -> Vec<f64> {
    for frame in 0..warmup {
        if dirty {
            dirty_transforms(scene, frame);
        }
        scene.app.update();
    }
    let mut samples = Vec::with_capacity(frames);
    for frame in warmup..warmup + frames {
        if dirty {
            dirty_transforms(scene, frame);
        }
        let start = Instant::now();
        scene.app.update();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples
}

#[derive(Clone, Copy)]
struct Stats {
    mean: f64,
    median: f64,
    p05: f64,
    p95: f64,
}

fn stats(samples: &[f64]) -> Stats {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let percentile = |p: f64| {
        let index = (p * (sorted.len() - 1) as f64).round() as usize;
        sorted[index]
    };
    Stats {
        mean,
        median: percentile(0.5),
        p05: percentile(0.05),
        p95: percentile(0.95),
    }
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let number = |index: usize, default: usize| -> usize {
        args.get(index)
            .map_or(default, |value| value.parse().expect("numeric argument"))
    };
    let instances = number(0, 4);
    let repetitions = number(1, 3);
    let warmup = number(2, 100);
    let frames = number(3, 600);
    let reverse = number(4, 0) != 0;

    let mut arms = vec![
        Arm::Default,
        Arm::Optout,
        Arm::Marker,
        Arm::Deferred,
        Arm::DeferredChanged,
    ];
    if reverse {
        arms.reverse();
    }
    println!(
        "config: model={MODEL} instances={instances} reps={repetitions} warmup={warmup} frames={frames} arm_order={}",
        arms.iter().map(|arm| arm.label()).collect::<Vec<_>>().join(",")
    );

    let mut active = vec![Vec::new(); arms.len()];
    let mut idle = vec![Vec::new(); arms.len()];
    for rep in 0..repetitions {
        for (index, arm) in arms.iter().enumerate() {
            let mut scene = build_scene(*arm, instances);
            if rep == 0 {
                println!(
                    "scene arm={} transforms={} spring_roots={} spring_joints={} dirty_targets={} stale_marked={}",
                    arm.label(),
                    scene.transform_count,
                    scene.spring_root_count,
                    scene.spring_joint_count,
                    scene.dirty.len(),
                    scene.stale.len()
                );
            }
            let active_samples = timed_frames(&mut scene, warmup, frames, true);
            let active_gate = scene.app.world().get_resource::<GateStats>().copied();
            let idle_samples = timed_frames(&mut scene, warmup, frames, false);
            let idle_gate = scene.app.world().get_resource::<GateStats>().copied();
            let active_stats = stats(&active_samples);
            let idle_stats = stats(&idle_samples);
            println!(
                "rep={rep} arm={} active_mean_ms={:.4} active_median_ms={:.4} active_p05_ms={:.4} active_p95_ms={:.4} idle_mean_ms={:.4} idle_median_ms={:.4}",
                arm.label(),
                active_stats.mean,
                active_stats.median,
                active_stats.p05,
                active_stats.p95,
                idle_stats.mean,
                idle_stats.median,
            );
            if let (Some(before), Some(after)) = (active_gate, idle_gate) {
                println!(
                    "rep={rep} gate=active ran={} skipped={} idle ran={} skipped={}",
                    before.ran,
                    before.skipped,
                    after.ran - before.ran,
                    after.skipped - before.skipped,
                );
            }
            active[index].push(active_samples);
            idle[index].push(idle_samples);
        }
    }

    let mut medians = Vec::new();
    for (index, arm) in arms.iter().enumerate() {
        let active_medians = active[index]
            .iter()
            .map(|samples| stats(samples).median)
            .collect::<Vec<_>>();
        let active_means = active[index]
            .iter()
            .map(|samples| stats(samples).mean)
            .collect::<Vec<_>>();
        let idle_medians = idle[index]
            .iter()
            .map(|samples| stats(samples).median)
            .collect::<Vec<_>>();
        let median = stats(&active_medians).median;
        medians.push(median);
        println!(
            "summary arm={} reps={} active_median_of_frame_medians_ms={:.4} active_median_of_frame_means_ms={:.4} active_spread_ms={:.4}..{:.4} idle_median_of_frame_medians_ms={:.4} idle_spread_ms={:.4}..{:.4}",
            arm.label(),
            repetitions,
            median,
            stats(&active_means).median,
            active_medians.iter().copied().fold(f64::INFINITY, f64::min),
            active_medians.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            stats(&idle_medians).median,
            idle_medians.iter().copied().fold(f64::INFINITY, f64::min),
            idle_medians.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        );
    }
    let median_of = |arm: Arm| medians[arms.iter().position(|candidate| *candidate == arm).unwrap()];
    println!(
        "delta deferred_vs_default_ms={:+.4} deferred_vs_optout_ms={:+.4} deferred_vs_marker_ms={:+.4} marker_vs_optout_ms={:+.4}",
        median_of(Arm::Deferred) - median_of(Arm::Default),
        median_of(Arm::Deferred) - median_of(Arm::Optout),
        median_of(Arm::Deferred) - median_of(Arm::Marker),
        median_of(Arm::Marker) - median_of(Arm::Optout),
    );
}
