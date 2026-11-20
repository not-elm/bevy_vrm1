use bevy::app::{App, PluginGroupBuilder, PostUpdate};
use bevy::asset::AssetServer;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::WgpuSettings;
use bevy::time::TimeUpdateStrategy;
use bevy::transform::TransformSystems;
use bevy_vrm1::prelude::*;
use std::time::{Duration, Instant};

const MODEL: &str = "vrm/AliciaSolid.vrm";

#[derive(Resource, Clone, Copy)]
struct BenchFlags {
    chain1: bool,
    chain2: bool,
    default_prop: bool,
}

fn chain1_enabled(flags: Res<BenchFlags>) -> bool {
    flags.chain1
}

fn chain2_enabled(flags: Res<BenchFlags>) -> bool {
    flags.chain2
}

fn default_prop_enabled(flags: Res<BenchFlags>) -> bool {
    flags.default_prop
}

#[derive(Resource)]
struct SegmentClock {
    stamps: [Option<Instant>; 7],
}

impl Default for SegmentClock {
    fn default() -> Self {
        Self { stamps: [None; 7] }
    }
}

fn stamp(
    mut clock: ResMut<SegmentClock>,
    index: usize,
) {
    clock.stamps[index] = Some(Instant::now());
}

fn stamp_entry(clock: ResMut<SegmentClock>) {
    stamp(clock, 0);
}

fn stamp_constraints(clock: ResMut<SegmentClock>) {
    stamp(clock, 1);
}

fn stamp_gaze(clock: ResMut<SegmentClock>) {
    stamp(clock, 2);
}

fn stamp_expressions(clock: ResMut<SegmentClock>) {
    stamp(clock, 3);
}

fn stamp_pre_spring(clock: ResMut<SegmentClock>) {
    stamp(clock, 4);
}

fn stamp_post_spring(clock: ResMut<SegmentClock>) {
    stamp(clock, 5);
}

fn stamp_tail(clock: ResMut<SegmentClock>) {
    stamp(clock, 6);
}

const SEGMENT_NAMES: [&str; 6] = [
    "entry_to_constraints",
    "chain1_span",
    "gaze_control_span",
    "chain2_span",
    "spring_bone_span",
    "post_spring_tail",
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

struct Scene {
    app: App,
    dirty: Vec<Entity>,
    extra_dirty: Vec<Entity>,
    transform_count: usize,
    spring_root_count: usize,
    spring_joint_count: usize,
}

fn build_scene(
    flags: BenchFlags,
    segment_markers: bool,
    instances: usize,
    extra_transforms: usize,
) -> Scene {
    let mut app = App::new();
    app.insert_resource(flags);
    app.insert_resource(SkipPostSpringPropagation);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        16,
    )));
    app.add_plugins((headless_default_plugins(), VrmPlugin));
    app.configure_sets(
        PostUpdate,
        VrmSystemSets::PropagateAfterConstraints.run_if(chain1_enabled),
    );
    app.configure_sets(
        PostUpdate,
        VrmSystemSets::PropagateAfterExpressions.run_if(chain2_enabled),
    );
    app.configure_sets(
        PostUpdate,
        TransformSystems::Propagate.run_if(default_prop_enabled),
    );
    if segment_markers {
        app.insert_resource(SegmentClock::default());
        app.add_systems(PostUpdate, stamp_entry.before(VrmSystemSets::Constraints));
        app.add_systems(
            PostUpdate,
            stamp_constraints
                .after(VrmSystemSets::Constraints)
                .before(VrmSystemSets::PropagateAfterConstraints),
        );
        app.add_systems(
            PostUpdate,
            stamp_gaze
                .after(VrmSystemSets::GazeControl)
                .before(VrmSystemSets::Expressions),
        );
        app.add_systems(
            PostUpdate,
            stamp_expressions
                .after(VrmSystemSets::Expressions)
                .before(VrmSystemSets::PropagateAfterExpressions),
        );
        app.add_systems(
            PostUpdate,
            stamp_pre_spring
                .after(VrmSystemSets::PropagateAfterExpressions)
                .before(VrmSystemSets::SpringBone),
        );
        app.add_systems(
            PostUpdate,
            stamp_post_spring.after(VrmSystemSets::SpringBone),
        );
        app.add_systems(PostUpdate, stamp_tail.after(stamp_post_spring));
    }

    let handles: Vec<Handle<Gltf>> = (0..instances)
        .map(|i| {
            let handle = app.world().resource::<AssetServer>().load::<Gltf>(MODEL);
            app.world_mut().spawn((
                VrmHandle(handle.clone()),
                Transform::from_xyz(i as f32 * 2.0, 0.0, 0.0),
            ));
            handle
        })
        .collect();
    let mut extra_dirty = Vec::with_capacity(extra_transforms / 2);
    for pair in 0..extra_transforms / 2 {
        let parent = app
            .world_mut()
            .spawn((
                Transform::from_xyz(pair as f32 * 0.01, 0.0, 0.0),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut().spawn((
            Transform::default(),
            GlobalTransform::default(),
            ChildOf(parent),
        ));
        extra_dirty.push(parent);
    }
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

    let (spring_roots, spring_joints) = {
        let mut query = app.world_mut().query_filtered::<Entity, With<SpringRoot>>();
        let roots = query.iter(app.world()).collect::<Vec<_>>();
        let joints = roots
            .iter()
            .flat_map(|root| {
                app.world()
                    .get::<SpringRoot>(*root)
                    .unwrap()
                    .joints
                    .0
                    .iter()
                    .copied()
            })
            .collect::<Vec<_>>();
        (roots, joints)
    };
    assert!(!spring_roots.is_empty(), "the model has no spring roots");
    let mut dirty = spring_roots
        .iter()
        .filter_map(|root| {
            let first = app
                .world()
                .get::<SpringRoot>(*root)?
                .joints
                .0
                .first()
                .copied()?;
            let parent = app.world().get::<ChildOf>(first)?.parent();
            (!spring_joints.contains(&parent)).then_some(parent)
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
    let transform_count = app
        .world_mut()
        .query_filtered::<Entity, With<Transform>>()
        .iter(app.world())
        .count();

    Scene {
        app,
        dirty,
        extra_dirty,
        transform_count,
        spring_root_count: spring_roots.len(),
        spring_joint_count: spring_joints.len(),
    }
}

fn dirty_transforms(
    scene: &mut Scene,
    frame: usize,
) {
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
    for entity in &scene.extra_dirty {
        scene
            .app
            .world_mut()
            .entity_mut(*entity)
            .get_mut::<Transform>()
            .expect("the extra transform keeps its Transform")
            .rotation = rotation;
    }
}

fn timed_frames(
    scene: &mut Scene,
    warmup: usize,
    frames: usize,
) -> Vec<f64> {
    for frame in 0..warmup {
        dirty_transforms(scene, frame);
        scene.app.update();
    }
    let mut samples = Vec::with_capacity(frames);
    for frame in warmup..warmup + frames {
        dirty_transforms(scene, frame);
        let start = Instant::now();
        scene.app.update();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples
}

fn timed_segments(
    scene: &mut Scene,
    warmup: usize,
    frames: usize,
) -> Vec<[f64; 6]> {
    for frame in 0..warmup {
        dirty_transforms(scene, frame);
        scene.app.update();
    }
    let mut samples = Vec::with_capacity(frames);
    for frame in warmup..warmup + frames {
        dirty_transforms(scene, frame);
        scene.app.update();
        let stamps = scene.app.world().resource::<SegmentClock>().stamps;
        let mut spans = [0.0; 6];
        for (index, span) in spans.iter_mut().enumerate() {
            let start = stamps[index].expect("segment start stamp");
            let end = stamps[index + 1].expect("segment end stamp");
            *span = end.duration_since(start).as_secs_f64() * 1000.0;
        }
        samples.push(spans);
    }
    samples
}

#[derive(Clone, Copy)]
struct Stats {
    median: f64,
    mean: f64,
    p05: f64,
    p95: f64,
}

fn stats(samples: &[f64]) -> Stats {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |p: f64| sorted[(p * (sorted.len() - 1) as f64).round() as usize];
    Stats {
        median: percentile(0.5),
        mean: samples.iter().sum::<f64>() / samples.len() as f64,
        p05: percentile(0.05),
        p95: percentile(0.95),
    }
}

fn median(values: &[f64]) -> f64 {
    stats(values).median
}

const ARMS: [(&str, BenchFlags); 8] = [
    (
        "baseline",
        BenchFlags {
            chain1: true,
            chain2: true,
            default_prop: true,
        },
    ),
    (
        "chain1_off",
        BenchFlags {
            chain1: false,
            chain2: true,
            default_prop: true,
        },
    ),
    (
        "chain2_off",
        BenchFlags {
            chain1: true,
            chain2: false,
            default_prop: true,
        },
    ),
    (
        "both_off",
        BenchFlags {
            chain1: false,
            chain2: false,
            default_prop: true,
        },
    ),
    (
        "no_default",
        BenchFlags {
            chain1: true,
            chain2: true,
            default_prop: false,
        },
    ),
    (
        "no_default_chain1_off",
        BenchFlags {
            chain1: false,
            chain2: true,
            default_prop: false,
        },
    ),
    (
        "no_default_chain2_off",
        BenchFlags {
            chain1: true,
            chain2: false,
            default_prop: false,
        },
    ),
    (
        "no_default_both_off",
        BenchFlags {
            chain1: false,
            chain2: false,
            default_prop: false,
        },
    ),
];

fn env_number(
    name: &str,
    default: usize,
) -> usize {
    std::env::var(name).ok().map_or(default, |value| {
        value.parse().expect("numeric environment value")
    })
}

#[test]
#[ignore = "measurement bench: run with cargo test --profile bench --bench chains12_bench -- --ignored --nocapture"]
fn measure_chains12() {
    let instances = env_number("CHAINS12_INSTANCES", 4);
    let repetitions = env_number("CHAINS12_REPS", 3);
    let warmup = env_number("CHAINS12_WARMUP", 100);
    let frames = env_number("CHAINS12_FRAMES", 600);
    let extra_transforms = env_number("CHAINS12_EXTRA_TRANSFORMS", 0);

    println!(
        "config: model={MODEL} instances={instances} extra_transforms={extra_transforms} reps={repetitions} warmup={warmup} frames={frames} chain3=off gating=configure_sets(VrmSystemSets::PropagateAfter{{Constraints,Expressions}}, TransformSystems::Propagate .run_if(resource))"
    );

    let mut arm_samples: [Vec<Vec<f64>>; 8] = Default::default();
    let mut segment_samples: [Vec<Vec<[f64; 6]>>; 8] = Default::default();
    let mut scene_info = String::new();
    for rep in 0..repetitions {
        for (arm, (name, flags)) in ARMS.iter().enumerate() {
            let mut scene = build_scene(*flags, false, instances, extra_transforms);
            if scene_info.is_empty() {
                scene_info = format!(
                    "scene: transforms={} spring_roots={} spring_joints={} dirty_targets={} extra_dirty={}",
                    scene.transform_count,
                    scene.spring_root_count,
                    scene.spring_joint_count,
                    scene.dirty.len(),
                    scene.extra_dirty.len()
                );
                println!("{scene_info}");
            }
            let samples = timed_frames(&mut scene, warmup, frames);
            let frame_stats = stats(&samples);
            println!(
                "rep={rep} arm={name} mean_ms={:.4} median_ms={:.4} p05_ms={:.4} p95_ms={:.4}",
                frame_stats.mean, frame_stats.median, frame_stats.p05, frame_stats.p95
            );
            arm_samples[arm].push(samples);

            let mut scene = build_scene(*flags, true, instances, extra_transforms);
            let spans = timed_segments(&mut scene, warmup, frames);
            segment_samples[arm].push(spans);
        }
    }

    println!("summary frame_ms");
    for (name, samples) in ARMS.iter().map(|(name, _)| name).zip(arm_samples.iter()) {
        let rep_medians = samples
            .iter()
            .map(|samples| median(samples))
            .collect::<Vec<_>>();
        println!(
            "arm={name} median_of_rep_medians_ms={:.4} rep_medians_ms={:?}",
            median(&rep_medians),
            rep_medians
        );
    }

    println!("summary segments_ms (median over frames, then median over reps)");
    for (arm, (name, _)) in ARMS.iter().enumerate() {
        for (segment, segment_name) in SEGMENT_NAMES.iter().enumerate() {
            let rep_medians = segment_samples[arm]
                .iter()
                .map(|reps| median(&reps.iter().map(|spans| spans[segment]).collect::<Vec<_>>()))
                .collect::<Vec<_>>();
            println!(
                "arm={name} segment={segment_name} median_ms={:.4} rep_medians_ms={:?}",
                median(&rep_medians),
                rep_medians
            );
        }
    }

    let frame_median = |arm: usize| {
        median(
            &arm_samples[arm]
                .iter()
                .map(|samples| median(samples))
                .collect::<Vec<_>>(),
        )
    };
    let baseline = frame_median(0);
    let no_default = frame_median(4);
    for (arm, (name, _)) in ARMS.iter().enumerate().skip(1) {
        let reference = if arm >= 4 { no_default } else { baseline };
        let saving = reference - frame_median(arm);
        println!(
            "delta arm={name} reference={} vs_reference_ms={:.4} vs_reference_pct={:.2}",
            if arm >= 4 { "no_default" } else { "baseline" },
            saving,
            100.0 * saving / reference
        );
    }
}
