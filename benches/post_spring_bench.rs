use bevy::app::{App, PluginGroupBuilder};
use bevy::asset::AssetServer;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::settings::WgpuSettings;
use bevy::time::TimeUpdateStrategy;
use bevy_vrm1::prelude::*;
use std::time::{Duration, Instant};

const MODEL: &str = "vrm/AliciaSolid.vrm";

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
    transform_count: usize,
    spring_root_count: usize,
    spring_joint_count: usize,
}

fn build_scene(
    skip_post_spring: bool,
    instances: usize,
) -> Scene {
    let mut app = App::new();
    app.add_plugins((headless_default_plugins(), VrmPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        16,
    )));
    if skip_post_spring {
        app.insert_resource(SkipPostSpringPropagation);
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

#[derive(Clone, Copy)]
struct Stats {
    mean: f64,
    median: f64,
    min: f64,
    max: f64,
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
        min: sorted[0],
        max: sorted[sorted.len() - 1],
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

    println!(
        "config: model={MODEL} instances={instances} reps={repetitions} warmup={warmup} frames={frames} arm_order=default,optout"
    );

    let mut arms: [Vec<Vec<f64>>; 2] = [Vec::new(), Vec::new()];
    let mut scene_info = String::new();
    for rep in 0..repetitions {
        for (arm, skip) in [(0usize, false), (1usize, true)] {
            let mut scene = build_scene(skip, instances);
            if scene_info.is_empty() {
                scene_info = format!(
                    "scene: transforms={} spring_roots={} spring_joints={} dirty_targets={}",
                    scene.transform_count,
                    scene.spring_root_count,
                    scene.spring_joint_count,
                    scene.dirty.len()
                );
                println!("{scene_info}");
            }
            let samples = timed_frames(&mut scene, warmup, frames);
            let arm_stats = stats(&samples);
            println!(
                "rep={rep} arm={} mean_ms={:.4} median_ms={:.4} p05_ms={:.4} p95_ms={:.4} min_ms={:.4} max_ms={:.4} fps_mean={:.2}",
                if skip { "optout" } else { "default" },
                arm_stats.mean,
                arm_stats.median,
                arm_stats.p05,
                arm_stats.p95,
                arm_stats.min,
                arm_stats.max,
                1000.0 / arm_stats.mean,
            );
            arms[arm].push(samples);
        }
    }

    for (arm, name) in [(0, "default"), (1, "optout")] {
        let medians = arms[arm]
            .iter()
            .map(|samples| stats(samples).median)
            .collect::<Vec<_>>();
        let means = arms[arm]
            .iter()
            .map(|samples| stats(samples).mean)
            .collect::<Vec<_>>();
        let fps = means.iter().map(|mean| 1000.0 / mean).collect::<Vec<_>>();
        println!(
            "summary arm={name} reps={} median_of_frame_medians_ms={:.4} median_of_frame_means_ms={:.4} spread_of_frame_medians_ms={:.4}..{:.4} median_fps={:.2} spread_fps={:.2}..{:.2}",
            repetitions,
            stats(&medians).median,
            stats(&means).median,
            medians.iter().copied().fold(f64::INFINITY, f64::min),
            medians.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            stats(&fps).median,
            fps.iter().copied().fold(f64::INFINITY, f64::min),
            fps.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        );
    }
}
