//! VRM glTF support: extension schemas, `MToon` material conversion, and the
//! glTF-pipeline loader ([`VrmGltfPlugin`]) that hooks VRM handling into the
//! stock Bevy glTF loader through
//! [`GltfExtensionHandler`](bevy::gltf::extensions::GltfExtensionHandler).
//!
//! The point of the extension handler is that every VRM-specific component is
//! written into the scene [`WorldAsset`](bevy::world_serialization::WorldAsset)
//! *while the file loads*, instead of being patched in over several runtime
//! frames. `bevy_gltf` calls `on_scene_completed` at
//! `crates/bevy_gltf/src/loader/mod.rs:1112-1120` and only then freezes the
//! world with `WorldAsset::new(world)` at `:1122`, which is what makes writing
//! into `scene_world` legal. A scene carries the crate-private `VrmSceneRoot`
//! marker, and the entity spawned with
//! [`VrmHandle`](crate::prelude::VrmHandle) binds that data onto itself and
//! becomes the public, initialized VRM root.
//!
//! # Exactly one loader claims `.vrm`
//!
//! [`VrmLoader`] below is the only loader that registers the `vrm` extension,
//! and it is added by this plugin. Bevy's `AssetServer` resolves a loader by the
//! asset's [`TypeId`] first and only then by extension, so
//! `asset_server.load("….vrm")` lands here and the scene comes out a
//! [`Gltf`](bevy::gltf::Gltf) whose VRM components are baked in.
//!
//! # `.vrma` is *not* claimed
//!
//! [`VrmLoader::extensions`] returns only `vrm`. `.vrma` belongs to
//! [`VrmaLoaderPlugin`](crate::vrma::VrmaLoaderPlugin), which loads it into a
//! [`VrmaAsset`](crate::prelude::VrmaAsset); two loaders claiming one extension
//! makes `AssetServer` refuse the app at registration time. So a `.vrma` still
//! takes [`VrmaHandle`](crate::prelude::VrmaHandle), while the `.vrm` side of the
//! pair is this pipeline.
//!
//! # How this is tested
//!
//! There is deliberately **no** end-to-end test that loads `assets/vrm/Elmer.vrm`
//! through `AssetServer` and inspects the resulting `WorldAsset`. `AssetServer`
//! loads asynchronously on bevy's IO task pool, and that task never completes
//! inside a `cargo test` binary in this crate's headless configuration: the
//! handle stays in [`LoadState::Loading`](bevy::asset::LoadState::Loading), no
//! failure event is emitted, and instrumentation placed in [`VrmLoader::load`]
//! and in `on_root` shows neither is ever entered - the wait ends in the
//! `bevy_asset` file reader, before any VRM code runs. The coverage is therefore
//! split:
//!
//! * [`handler::root`] parses the real `Elmer.vrm` and `AliciaSolid.vrm` bytes
//!   with the `gltf` crate and drives the same `on_root` entry point;
//! * [`handler::scene`], [`handler::first_person`], [`handler::materials`] and
//!   [`handler::nodes`] each test their own step on hand-built worlds.
//!
//! `examples/simple.rs` is the end-to-end case, and it needs a window.

pub mod extensions;
pub mod handler;
pub mod materials;

pub mod prelude {
    pub use crate::vrm::gltf::{
        VrmGltfPlugin, VrmLoader,
        extensions::{VrmExtensions, VrmNode, vrmc_spring_bone::*, vrmc_vrm::*},
        materials::*,
    };
}

use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::app::AnimationSystems;
use bevy::asset::io::Reader;
use bevy::asset::{AssetApp, AssetLoader, LoadContext};
use bevy::gltf::extensions::GltfExtensionHandlers;
use bevy::gltf::{
    DefaultGltfImageSampler, Gltf, GltfError, GltfLoader, GltfLoaderSettings, GltfPlugin,
};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use std::sync::{Arc, Mutex};

use crate::prelude::MtoonMaterialPlugin;
use crate::prelude::{
    ColliderShape, ConstraintExecutionOrder, EffectiveExpressionWeights, ExpressionMorphBinds,
    ExpressionSettings, Initialized, LookAtProperties, LookAtType, MorphBind,
    PendingNodeConstraint, RestGlobalTransform, RestTransform, RestWorldTransform, Vrm, VrmBone,
    VrmConstraintKind, VrmExpressionWeights, VrmHeadOnly, VrmNodeConstraint, VrmNodeIndex, VrmPath,
    apply_expression_morph_binds,
};
use crate::system_set::VrmSystemSets;
use crate::vrm::gltf::handler::VrmExtensionHandler;
use crate::vrm::humanoid_bone::register_bone_components;
use crate::vrm::instance::VrmSceneRoot;

/// Registers the VRM extension handler and the `.vrm` asset loader.
///
/// Must be added **after** `DefaultPlugins`:
///
/// * the handler must be registered after `GltfPlugin` and `PbrPlugin`, because
///   `on_spawn_mesh_and_material` swaps the `StandardMaterial` that
///   `GltfExtensionHandlerPbr` has already put on the entity, and
/// * `finish` reuses resources inserted by `GltfPlugin::finish`
///   ([`DefaultGltfImageSampler`], [`GltfExtensionHandlers`]).
///
/// # Which plugins an app needs
///
/// This plugin is the *loading* half: it claims the `vrm` extension and writes
/// every VRM component into the scene
/// [`WorldAsset`](bevy::world_serialization::WorldAsset) while the file loads.
/// The runtime systems live in [`VrmPlugin`](crate::vrm::VrmPlugin), so a
/// complete app needs both:
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy_vrm1::prelude::*;
/// # let mut app = App::new();
/// app.add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin));
/// ```
///
/// * `VrmGltfPlugin` — this plugin; loads the `.vrm` and writes the components.
/// * `VrmPlugin` — spring bones, gaze control, expressions, node constraints and
///   `MToon`. Required for anything to move.
/// * `VrmaPlugin` — **optional**, only for `.vrma` playback. It owns the
///   animation-graph build, the retarget and
///   [`PlayVrma`](crate::prelude::PlayVrma). A pipeline scene carries everything
///   that path needs (see `handler::scene`), so a child
///   [`VrmaHandle`](crate::prelude::VrmaHandle) plus `PlayVrma` on
///   [`LoadedVrma`](crate::prelude::LoadedVrma) is enough.
///
/// It also adds [`MtoonMaterialPlugin`], because the loader creates labeled
/// `MToonMaterial` assets, and
/// [`VrmGltfRuntimePlugin`](crate::vrm::runtime::VrmGltfRuntimePlugin), which
/// evaluates the node constraints this loader attaches.
#[derive(Default)]
pub struct VrmGltfPlugin;

impl Plugin for VrmGltfPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        assert!(
            app.is_plugin_added::<GltfPlugin>(),
            "VrmGltfPlugin must be added after DefaultPlugins (GltfPlugin/PbrPlugin)",
        );

        // The loader creates labeled `MToonMaterial` assets. `VrmPlugin` also
        // adds this plugin, hence the guard.
        if !app.is_plugin_added::<MtoonMaterialPlugin>() {
            app.add_plugins(MtoonMaterialPlugin);
        }

        // Guarded so `VrmPlugin + VrmGltfPlugin` apps do not add it twice.
        if !app.is_plugin_added::<crate::vrm::runtime::VrmGltfRuntimePlugin>() {
            app.add_plugins(crate::vrm::runtime::VrmGltfRuntimePlugin);
        }

        // Everything written into the scene `WorldAsset` must be registered:
        // the scene spawn pipeline copies components out of the asset through
        // reflection, and drops the ones whose type is unknown
        // (`ReflectComponent::apply_or_insert_mapped`).
        app.register_type::<Vrm>()
            .register_type::<Initialized>()
            .register_type::<VrmBone>()
            .register_type::<RestTransform>()
            .register_type::<RestGlobalTransform>()
            .register_type::<RestWorldTransform>()
            .register_type::<VrmNodeIndex>()
            .register_type::<VrmHeadOnly>()
            .register_type::<PendingNodeConstraint>()
            .register_type::<VrmNodeConstraint>()
            .register_type::<VrmConstraintKind>()
            .register_type::<ConstraintExecutionOrder>()
            // The source asset path of a pipeline scene. `VrmPlugin` registers it
            // too; double registration is a no-op, and a scene that is loaded
            // without `VrmPlugin` in the app must still carry it.
            .register_type::<VrmPath>()
            // Written on the root bone and every humanoid bone so the VRMA
            // retarget can recognise a model as a pose source. `VrmaPlugin`
            // registers it too; double registration is a no-op.
            .register_type::<crate::vrma::RetargetSource>()
            .register_type::<LookAtProperties>()
            .register_type::<LookAtType>()
            // Spring bones. Double registration with `VrmSpringBonePlugin` is a
            // no-op.
            .register_type::<crate::vrm::spring_bone::SpringRoot>()
            .register_type::<crate::vrm::spring_bone::SpringJoints>()
            .register_type::<crate::vrm::spring_bone::SpringColliders>()
            .register_type::<crate::vrm::spring_bone::SpringCenterNode>()
            .register_type::<crate::vrm::spring_bone::SpringJointProps>()
            .register_type::<crate::vrm::spring_bone::SpringJointState>()
            .register_type::<ColliderShape>()
            // The expression property types of `vrma::animation::properties`:
            // `apply_expression_morph_binds` reads the first three off the
            // root and publishes `EffectiveExpressionWeights` into the fourth.
            .register_type::<VrmExpressionWeights>()
            .register_type::<EffectiveExpressionWeights>()
            .register_type::<ExpressionMorphBinds>()
            .register_type::<ExpressionSettings>()
            .register_type::<MorphBind>()
            // The private marker baked onto the scene root by
            // `handler::scene::mark_initialized`. An unregistered component is
            // dropped by `ReflectComponent::apply_or_insert_mapped` during scene
            // instantiation, so the attach observer would never see it.
            .register_type::<VrmSceneRoot>()
            // The animation types `handler::scene` writes onto the root bone and
            // the scene root (`AnimationPlayer`, `AnimatedBy`, and one
            // `AnimationTargetId` per humanoid node). Bevy registers none of these
            // in `AnimationPlugin`, so a `.vrm` scene would panic the
            // `WorldInstance` spawner without this. Double registration is a
            // no-op.
            .register_type::<AnimationPlayer>()
            .register_type::<AnimationTransitions>()
            .register_type::<AnimatedBy>()
            .register_type::<AnimationTargetId>();

        // The bone markers written on the bone entities and the
        // `<Bone>BoneEntity` holders written on the scene root
        // (`handler::scene::insert_humanoid_bone_holders`). Same reason: an
        // unregistered component is dropped when the scene is instantiated.
        register_bone_components(app);

        // `GltfConvertCoordinates` is a public type with public bool fields, so
        // only the two flags this crate cares about are read.
        let (default_rotate_scene_entity, default_rotate_meshes) = app
            .get_added_plugins::<GltfPlugin>()
            .first()
            .map(|gltf_plugin| {
                (
                    gltf_plugin.convert_coordinates.rotate_scene_entity,
                    gltf_plugin.convert_coordinates.rotate_meshes,
                )
            })
            .unwrap_or((false, false));

        let handler = VrmExtensionHandler {
            default_rotate_scene_entity,
            default_rotate_meshes,
            ..Default::default()
        };

        // The registry is shared by every glTF loader in the app; the handler
        // no-ops for non-VRM files.
        #[cfg(target_family = "wasm")]
        bevy::tasks::block_on(async {
            app.world_mut()
                .resource_mut::<GltfExtensionHandlers>()
                .0
                .write()
                .await
                .push(Box::new(handler));
        });
        #[cfg(not(target_family = "wasm"))]
        app.world_mut()
            .resource_mut::<GltfExtensionHandlers>()
            .0
            .write_blocking()
            .push(Box::new(handler));

        app.preregister_asset_loader::<VrmLoader>(&["vrm"]);

        add_expression_bind_pass(app);
    }

    fn finish(
        &self,
        app: &mut App,
    ) {
        // Assemble the inner `GltfLoader` from the same resources as the stock
        // loader, sharing the handler registry `Arc`.
        let supported_compressed_formats = app
            .world()
            .get_resource::<CompressedImageFormatSupport>()
            .map(|support| support.0)
            .unwrap_or(CompressedImageFormats::NONE);

        let default_sampler = app
            .world()
            .get_resource::<DefaultGltfImageSampler>()
            .map(|sampler| sampler.get_internal())
            // `GltfPlugin::finish` has already run for any correctly ordered
            // app; this fallback only exists so a hand-built app does not panic.
            .unwrap_or_else(|| Arc::new(Mutex::new(ImageSamplerDescriptor::default())));

        let extensions = app
            .world()
            .get_resource::<GltfExtensionHandlers>()
            .map(|handlers| handlers.0.clone())
            .unwrap_or_default();

        let default_convert_coordinates = app
            .get_added_plugins::<GltfPlugin>()
            .first()
            .map(|gltf_plugin| gltf_plugin.convert_coordinates)
            .unwrap_or_default();

        app.register_asset_loader(VrmLoader(GltfLoader {
            supported_compressed_formats,
            custom_vertex_attributes: Default::default(),
            default_sampler,
            default_convert_coordinates,
            extensions,
            // Left at its `Default`: `GltfSkinnedMeshBoundsPolicy::default()`
            // is already `Dynamic` (`bevy_gltf/src/lib.rs:213-214`), which is
            // what a skinned avatar needs — rest-pose bounds would cull a mesh
            // once spring bones and animation move it.
            default_skinned_mesh_bounds_policy: Default::default(),
        }));
    }
}

/// Schedules the load-time expression pass.
///
/// `apply_expression_morph_binds` reads [`VrmExpressionWeights`] /
/// [`ExpressionMorphBinds`] / [`ExpressionSettings`], the three components
/// [`handler::scene::build_expressions`] writes onto the scene root, and it is
/// required for an avatar with **no** `.vrma` in the app at all — an avatar whose
/// expressions are driven by
/// [`ExpressionWeightProperty`](crate::vma::animation::ExpressionWeightProperty)
/// curves, by `VrmExpressionWeights` written directly (which is what the
/// [`SetExpressions`] / [`ModifyExpressions`] / [`ClearExpressions`] triggers do),
/// or by nothing more than the file's own defaults still needs its morph weights
/// distributed. Putting it under `VrmaPlugin` would make that depend on a plugin
/// whose whole job is animation.
///
/// # Where in the schedule
///
/// `VrmSystemSets::Expressions` is the VRM spec's expression step
/// (`https://vrm.dev/api/api_update/`); `VrmPlugin` chains the manual transform
/// propagation after it (`src/vrm.rs:161-168`), so a bind pass here is published
/// before anything downstream reads `GlobalTransform`. Two edges are pinned:
///
/// * `.after(AnimationSystems)` — the weights being distributed are written by
///   the animation graph during this frame's `PostUpdate`
///   (`bevy_animation/src/lib.rs:1305`). Without the edge the pass would apply
///   the *previous* frame's weights.
/// * `.after(VrmSystemSets::GazeControl)` — so an expression-driven gaze
///   contribution lands in the same frame as a gaze-driven expression.
fn add_expression_bind_pass(app: &mut App) {
    app.add_systems(
        PostUpdate,
        apply_expression_morph_binds
            .in_set(VrmSystemSets::Expressions)
            .after(AnimationSystems)
            .after(VrmSystemSets::GazeControl),
    );
}

/// A thin wrapper around the stock [`GltfLoader`] that registers the `vrm` file
/// extension. All glTF parsing is done by Bevy; everything VRM-specific lives
/// in [`VrmExtensionHandler`].
///
/// Load a scene out of it with
/// [`GltfAssetLabel::Scene`](bevy::gltf::GltfAssetLabel::Scene):
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
///
/// fn spawn(mut commands: Commands, asset_server: Res<AssetServer>) {
///     commands.spawn(WorldAssetRoot(
///         asset_server.load(GltfAssetLabel::Scene(0).from_asset("vrm/Elmer.vrm")),
///     ));
/// }
/// ```
#[derive(TypePath)]
pub struct VrmLoader(pub GltfLoader);

impl AssetLoader for VrmLoader {
    type Asset = Gltf;
    type Settings = GltfLoaderSettings;
    type Error = GltfError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &GltfLoaderSettings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Gltf, GltfError> {
        // `GltfLoaderSettings` does not implement `Clone`, so copy the fields.
        // Every field is passed through untouched:
        //
        // * `validate` keeps `GltfLoaderSettings`'s own default of `true`.
        //   VRM is a glTF profile, not a glTF dialect, so a `.vrm` is expected
        //   to validate; forcing it off would trade a precise error message for
        //   silently mis-parsed files.
        // * `include_source` is the caller's choice. The handler does **not**
        //   need it: every hook receives the parsed `gltf::Gltf` (or
        //   `gltf::Node` / `gltf::Material`) directly and reads
        //   `document.extensions()` off it, so the `Gltf::source` copy of the
        //   JSON is dead weight for this loader.
        let settings = GltfLoaderSettings {
            load_meshes: settings.load_meshes,
            load_materials: settings.load_materials,
            load_cameras: settings.load_cameras,
            load_lights: settings.load_lights,
            load_animations: settings.load_animations,
            include_source: settings.include_source,
            default_sampler: settings.default_sampler.clone(),
            override_sampler: settings.override_sampler,
            validate: settings.validate,
            convert_coordinates: settings.convert_coordinates,
            skinned_mesh_bounds_policy: settings.skinned_mesh_bounds_policy,
        };
        self.0.load(reader, &settings, load_context).await
    }

    fn extensions(&self) -> &[&str] {
        // `vrma` is deliberately absent: `VrmaLoaderPlugin` owns it, and
        // two loaders claiming one extension makes `AssetServer` refuse the app.
        &["vrm"]
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::{
        ExpressionMorphBinds, ExpressionSetting, ExpressionSettings, MorphBind, MorphBindTable,
        Vrm, VrmExpressionWeights,
    };
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::expressions::{ExpressionCategory, ExpressionOverrideType};
    use bevy::asset::AssetLoader;
    use bevy::gltf::GltfLoader;
    use bevy::image::{CompressedImageFormats, ImageSamplerDescriptor};
    use bevy::mesh::morph::MorphWeights;
    use bevy::platform::collections::HashMap;
    use bevy::prelude::*;
    use std::sync::{Arc, Mutex};

    use super::{VrmGltfPlugin, VrmLoader, add_expression_bind_pass};

    /// A loader with the fields `VrmGltfPlugin::finish` fills; only
    /// `extensions()` is under test, so the rest is arbitrary.
    fn loader() -> VrmLoader {
        VrmLoader(GltfLoader {
            supported_compressed_formats: CompressedImageFormats::NONE,
            custom_vertex_attributes: Default::default(),
            default_sampler: Arc::new(Mutex::new(ImageSamplerDescriptor::default())),
            default_convert_coordinates: Default::default(),
            extensions: Default::default(),
            default_skinned_mesh_bounds_policy: Default::default(),
        })
    }

    /// The extension list is a *claim*: `vrma` belongs to
    /// `VrmaLoaderPlugin`, and two loaders claiming one extension makes
    /// `AssetServer` refuse the app at registration time.
    #[test]
    fn the_loader_claims_only_the_vrm_extension() -> TestResult {
        assert_eq!(loader().extensions(), &["vrm"]);
        success!()
    }

    /// A scene's expression data is written by the handler and has to be applied
    /// by a scheduled system: nothing else in the crate reads
    /// `VrmExpressionWeights`. This runs a whole `App` so the *schedule* is under
    /// test, not the system alone.
    ///
    /// `add_expression_bind_pass` is called directly instead of building
    /// [`VrmGltfPlugin`], whose `MtoonMaterialPlugin` half needs render resources
    /// (`Assets<Shader>`, and more beyond) that a headless `cargo test` app has
    /// no business providing. `build` calls it unconditionally, on the last
    /// line.
    ///
    /// What the assertion covers is the *whole* path the load-time shape has
    /// left: the weights sit on the root, and this pass is the only thing that
    /// moves them, so one weight written on the root is one morph slot driven on
    /// the mesh with no intermediate entity. Which `.vrma` expression curves
    /// reach that root component is a separate question, answered by
    /// [`retarget_expression_curves`](crate::vrma::animation::expressions::retarget_expression_curves).
    #[test]
    fn the_pipeline_expression_binds_are_applied_by_the_schedule() -> TestResult {
        let mut app = test_app();
        add_expression_bind_pass(&mut app);

        let mesh = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0, 0.0], None)?)
            .id();
        let root = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(root).insert((
            Vrm,
            VrmExpressionWeights(HashMap::from([("happy".to_owned(), 0.5)])),
            ExpressionMorphBinds(MorphBindTable(HashMap::from([(
                "happy".to_owned(),
                vec![MorphBind {
                    target: mesh,
                    index: 1,
                    weight: 1.0,
                }],
            )]))),
            ExpressionSettings(HashMap::from([(
                "happy".to_owned(),
                ExpressionSetting {
                    is_binary: false,
                    category: ExpressionCategory::Other,
                    override_blink: ExpressionOverrideType::None,
                    override_look_at: ExpressionOverrideType::None,
                    override_mouth: ExpressionOverrideType::None,
                },
            )])),
        ));

        app.update();

        let morph = app.world().get::<MorphWeights>(mesh).unwrap();
        assert_eq!(morph.weights(), &[0.0, 0.5]);
        success!()
    }
}
