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

#[derive(Clone, Copy, PartialEq)]
enum Arm {
    NoCollider,
    SphereContact,
    CapsuleContact,
}

impl Arm {
    const fn name(self) -> &'static str {
        match self {
            Self::NoCollider => "no-collider",
            Self::SphereContact => "sphere-contact",
            Self::CapsuleContact => "capsule-contact",
        }
    }
}

struct Scene {
    app: App,
    dirty: Vec<Entity>,
    collider_count: usize,
}

fn build_scene(
    instances: usize,
    arm: Arm,
    colliders_per_root: usize,
) -> Scene {
    let mut app = App::new();
    app.add_plugins((headless_default_plugins(), VrmPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        16,
    )));

    let mut roots = Vec::new();
    for i in 0..instances {
        let handle = app.world().resource::<AssetServer>().load::<Gltf>(MODEL);
        roots.push(
            app.world_mut()
                .spawn((
                    VrmHandle(handle),
                    Transform::from_xyz(i as f32 * 2.0, 0.0, 0.0),
                ))
                .id(),
        );
    }
    app.finish();
    app.cleanup();

    let mut ready = false;
    for _ in 0..1200 {
        app.update();
        let initialized = app
            .world_mut()
            .query_filtered::<Entity, With<Initialized>>()
            .iter(app.world())
            .count();
        if initialized >= instances {
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

    let mut collider_count = 0;
    let spring_roots = {
        let mut query = app.world_mut().query_filtered::<Entity, With<SpringRoot>>();
        query.iter(app.world()).collect::<Vec<_>>()
    };
    assert!(!spring_roots.is_empty(), "the model has no spring roots");

    for root in spring_roots {
        let joints = app
            .world()
            .get::<SpringRoot>(root)
            .expect("the spring root exists")
            .joints
            .0
            .clone();
        let mut new_colliders = Vec::with_capacity(colliders_per_root);
        if arm == Arm::NoCollider {
            app.world_mut()
                .entity_mut(root)
                .get_mut::<SpringRoot>()
                .expect("the spring root exists")
                .colliders
                .0
                .clear();
            continue;
        }
        let (Some(head), Some(tail)) = (joints.first(), joints.get(1)) else {
            continue;
        };
        let Some(head_gtf) = app.world().get::<GlobalTransform>(*head) else {
            continue;
        };
        let Some(tail_gtf) = app.world().get::<GlobalTransform>(*tail) else {
            continue;
        };
        let head_pos = head_gtf.translation();
        let bone_length = head_pos.distance(tail_gtf.translation()).max(1e-3);
        for k in 0..colliders_per_root {
            let offset =
                Vec3::new(0.1 * (k as f32 + 1.0), 0.05 * (k as f32 + 1.0), 0.0) * bone_length;
            let center = head_pos + offset;
            let shape = match arm {
                Arm::SphereContact => ColliderShape::Sphere(bevy_vrm1::prelude::Sphere {
                    offset: [0.0; 3],
                    radius: 4.0 * bone_length,
                }),
                Arm::CapsuleContact => ColliderShape::Capsule(bevy_vrm1::prelude::Capsule {
                    offset: [0.0; 3],
                    radius: 4.0 * bone_length,
                    tail: [0.0, bone_length, 0.0],
                }),
                Arm::NoCollider => unreachable!("the no-collider arm is handled above"),
            };
            let collider = app
                .world_mut()
                .spawn((
                    Transform::default(),
                    GlobalTransform::from_translation(center),
                    shape,
                ))
                .id();
            new_colliders.push((collider, shape));
        }
        collider_count += new_colliders.len();
        app.world_mut()
            .entity_mut(root)
            .get_mut::<SpringRoot>()
            .expect("the spring root exists")
            .colliders
            .0
            .extend(new_colliders);
    }

    Scene {
        app,
        dirty: roots,
        collider_count,
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
    p05: f64,
    p95: f64,
    min: f64,
    max: f64,
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
        min: sorted[0],
        max: sorted[sorted.len() - 1],
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
    let colliders_per_root = number(4, 8);
    let arms = [Arm::NoCollider, Arm::SphereContact, Arm::CapsuleContact];

    println!(
        "config: model={MODEL} instances={instances} reps={repetitions} warmup={warmup} frames={frames} colliders_per_root={colliders_per_root} arm_order=no-collider,sphere-contact,capsule-contact"
    );

    let mut per_arm: Vec<Vec<Vec<f64>>> = vec![Vec::new(); arms.len()];
    let mut scene_info = String::new();
    for rep in 0..repetitions {
        for (arm_index, arm) in arms.iter().enumerate() {
            let mut scene = build_scene(instances, *arm, colliders_per_root);
            if scene_info.is_empty() {
                let transform_count = {
                    let mut query = scene
                        .app
                        .world_mut()
                        .query_filtered::<Entity, With<Transform>>();
                    query.iter(scene.app.world()).count()
                };
                let spring_root_count = {
                    let mut query = scene
                        .app
                        .world_mut()
                        .query_filtered::<Entity, With<SpringRoot>>();
                    query.iter(scene.app.world()).count()
                };
                scene_info =
                    format!("scene: transforms={transform_count} spring_roots={spring_root_count}");
                println!("{scene_info}");
            }
            let collider_count = scene.collider_count;
            let samples = timed_frames(&mut scene, warmup, frames);
            let arm_stats = stats(&samples);
            println!(
                "rep={rep} arm={} colliders={collider_count} mean_ms={:.4} median_ms={:.4} p05_ms={:.4} p95_ms={:.4} min_ms={:.4} max_ms={:.4} fps_mean={:.2}",
                arm.name(),
                arm_stats.mean,
                arm_stats.median,
                arm_stats.p05,
                arm_stats.p95,
                arm_stats.min,
                arm_stats.max,
                1000.0 / arm_stats.mean,
            );
            per_arm[arm_index].push(samples);
        }
    }

    for (arm, samples_by_rep) in arms.iter().zip(&per_arm) {
        let medians = samples_by_rep
            .iter()
            .map(|samples| stats(samples).median)
            .collect::<Vec<_>>();
        let means = samples_by_rep
            .iter()
            .map(|samples| stats(samples).mean)
            .collect::<Vec<_>>();
        let fps = means.iter().map(|mean| 1000.0 / mean).collect::<Vec<_>>();
        println!(
            "summary arm={} reps={} median_of_frame_medians_ms={:.4} median_of_frame_means_ms={:.4} spread_of_frame_medians_ms={:.4}..{:.4} median_fps={:.2} spread_fps={:.2}..{:.2}",
            arm.name(),
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
