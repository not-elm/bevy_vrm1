pub mod extensions;
pub mod handler;

pub mod materials;

pub mod prelude {
    pub use crate::vrm::gltf::{
        VrmLoader,
        extensions::{VrmExtensions, VrmNode, vrmc_spring_bone::*, vrmc_vrm::*},
        materials::*,
    };
}

use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::app::AnimationSystems;
use bevy::asset::io::Reader;
use bevy::asset::{AssetApp, AssetLoader, LoadContext};
use bevy::gltf::convert_coordinates::GltfConvertCoordinates;
use bevy::gltf::extensions::GltfExtensionHandlers;
use bevy::gltf::{
    DefaultGltfImageSampler, Gltf, GltfError, GltfLoader, GltfLoaderSettings, GltfPlugin,
};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use std::sync::{Arc, Mutex};

use crate::prelude::{
    ColliderShape, ConstraintExecutionOrder, EffectiveExpressionWeights, ExpressionMorphBinds,
    ExpressionSettings, Initialized, LookAtProperties, LookAtType, MorphBind,
    PendingNodeConstraint, RestGlobalTransform, RestTransform, RestWorldTransform, Vrm, VrmBone,
    VrmConstraintKind, VrmExpressionWeights, VrmHeadOnly, VrmNodeConstraint, VrmNodeIndex, VrmPath,
    apply_expression_morph_binds,
};
use crate::system_set::VrmSystemSets;
use crate::vrm::gltf::handler::VrmExtensionHandler;
use crate::vrm::instance::VrmSceneRoot;

/// Registers the VRM extension handler and the `.vrm` asset loader.
///
/// Added by [`VrmPlugin`](crate::prelude::VrmPlugin), after `DefaultPlugins`.
#[derive(Default)]
pub(crate) struct VrmGltfPlugin;

impl Plugin for VrmGltfPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        assert!(
            app.is_plugin_added::<GltfPlugin>(),
            "VrmPlugin must be added after DefaultPlugins (GltfPlugin/PbrPlugin)",
        );

        app.add_plugins(crate::vrm::runtime::VrmGltfRuntimePlugin);

        app.register_type::<Vrm>()
            .register_type::<Initialized>()
            .register_type::<VrmBone>()
            .register_type::<RestTransform>()
            .register_type::<RestGlobalTransform>()
            .register_type::<RestWorldTransform>()
            .register_type::<VrmNodeIndex>()
            .register_type::<VrmHeadOnly>()
            .register_type::<crate::vrm::components::FirstPersonView>()
            .register_type::<PendingNodeConstraint>()
            .register_type::<VrmNodeConstraint>()
            .register_type::<VrmConstraintKind>()
            .register_type::<ConstraintExecutionOrder>()
            .register_type::<VrmPath>()
            .register_type::<crate::vrma::RetargetSource>()
            .register_type::<LookAtProperties>()
            .register_type::<LookAtType>()
            .register_type::<crate::vrm::spring_bone::SpringRoot>()
            .register_type::<crate::vrm::spring_bone::SpringJoints>()
            .register_type::<crate::vrm::spring_bone::SpringColliders>()
            .register_type::<crate::vrm::spring_bone::SpringCenterNode>()
            .register_type::<crate::vrm::spring_bone::SpringJointProps>()
            .register_type::<crate::vrm::spring_bone::SpringJointState>()
            .register_type::<ColliderShape>()
            .register_type::<VrmExpressionWeights>()
            .register_type::<EffectiveExpressionWeights>()
            .register_type::<ExpressionMorphBinds>()
            .register_type::<ExpressionSettings>()
            .register_type::<MorphBind>()
            .register_type::<VrmSceneRoot>()
            .register_type::<AnimationPlayer>()
            .register_type::<AnimationTransitions>()
            .register_type::<AnimatedBy>()
            .register_type::<AnimationTargetId>();

        let handler = VrmExtensionHandler {
            default_convert_coordinates: app
                .get_added_plugins::<GltfPlugin>()
                .first()
                .map(|plugin| plugin.convert_coordinates)
                .unwrap_or_default(),
            ..Default::default()
        };

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
        let supported_compressed_formats = app
            .world()
            .get_resource::<CompressedImageFormatSupport>()
            .map(|support| support.0)
            .unwrap_or(CompressedImageFormats::NONE);

        let default_sampler = app
            .world()
            .get_resource::<DefaultGltfImageSampler>()
            .map(|sampler| sampler.get_internal())
            .unwrap_or_else(|| Arc::new(Mutex::new(ImageSamplerDescriptor::default())));

        let extensions = app
            .world()
            .get_resource::<GltfExtensionHandlers>()
            .map(|handlers| handlers.0.clone())
            .unwrap_or_default();

        // The application configures glTF loading on `GltfPlugin`; a `.vrm` is
        // a glTF and honours the same configuration as the stock loader does.
        // `default_convert_coordinates` only reaches non-`.vrm` loads that bevy
        // routes here; a `.vrm` always names its own (see `vrm_load_settings`).
        let (
            custom_vertex_attributes,
            default_convert_coordinates,
            default_skinned_mesh_bounds_policy,
        ) = app
            .get_added_plugins::<GltfPlugin>()
            .first()
            .map(|plugin| {
                (
                    plugin.custom_vertex_attributes.clone(),
                    plugin.convert_coordinates,
                    plugin.skinned_mesh_bounds_policy,
                )
            })
            .unwrap_or_default();

        app.register_asset_loader(VrmLoader(GltfLoader {
            supported_compressed_formats,
            custom_vertex_attributes,
            default_sampler,
            default_convert_coordinates,
            extensions,
            default_skinned_mesh_bounds_policy,
        }));
    }
}

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
/// extension and pins the settings a VRM depends on (source kept, no coordinate conversion).
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
        // `Gltf` has two loaders now, so bevy falls back to the last registered
        // one, this one, for a typed `load::<Gltf>` whose extension matches
        // neither exactly (`Ship.GLB`, an extensionless URL). Only a `.vrm` gets
        // the VRM settings; anything else loads exactly as the stock loader would.
        if !is_vrm_path(load_context.path()) {
            return self.0.load(reader, settings, load_context).await;
        }
        let mut gltf = self
            .0
            .load(reader, &vrm_load_settings(settings), load_context)
            .await?;
        release_binary_chunk(&mut gltf.source);
        Ok(gltf)
    }

    fn extensions(&self) -> &[&str] {
        &["vrm"]
    }
}

/// Whether `path` names a `.vrm` file, whatever the case of its extension.
fn is_vrm_path(path: &bevy::asset::AssetPath<'_>) -> bool {
    path.get_extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("vrm"))
}

/// Drops the GLB binary chunk from the kept source, leaving its JSON document.
///
/// [`VrmHandle`](crate::prelude::VrmHandle) keeps the `Gltf` alive for as long
/// as the avatar exists, and `include_source` makes that asset own a full copy
/// of the file's BIN chunk (vertex buffers and embedded textures, already
/// decoded into the mesh and image assets). Only the JSON extensions are read
/// back ([`VrmExtensions::from_gltf`](extensions::VrmExtensions::from_gltf)), so
/// the binary copy would be dead weight of the size of the file.
fn release_binary_chunk(source: &mut Option<::gltf::Gltf>) {
    if let Some(source) = source.as_mut() {
        source.blob = None;
    }
}

/// The caller's settings, except for the two a `.vrm` load depends on.
///
/// * The source is kept, so [`VrmExtensions::from_gltf`](extensions::VrmExtensions::from_gltf)
///   can read the VRM extensions off the loaded asset (its binary chunk is
///   released after the load, see [`release_binary_chunk`]).
/// * Coordinates are never converted, whatever
///   [`GltfPlugin::convert_coordinates`] says: rest poses, gaze and body
///   tracking assume the avatar's glTF +Z forward, and the `firstPerson: auto`
///   split rebuilds meshes from the raw accessors, which `rotate_meshes` would
///   leave 180° off their converted bind poses.
fn vrm_load_settings(settings: &GltfLoaderSettings) -> GltfLoaderSettings {
    // `GltfLoaderSettings` does not implement `Clone`, so copy the fields.
    GltfLoaderSettings {
        load_meshes: settings.load_meshes,
        load_materials: settings.load_materials,
        load_cameras: settings.load_cameras,
        load_lights: settings.load_lights,
        load_animations: settings.load_animations,
        include_source: true,
        default_sampler: settings.default_sampler.clone(),
        override_sampler: settings.override_sampler,
        validate: settings.validate,
        convert_coordinates: Some(GltfConvertCoordinates::default()),
        skinned_mesh_bounds_policy: settings.skinned_mesh_bounds_policy,
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
    use bevy::gltf::convert_coordinates::GltfConvertCoordinates;
    use bevy::gltf::{GltfLoader, GltfLoaderSettings};
    use bevy::image::{CompressedImageFormats, ImageSamplerDescriptor};
    use bevy::mesh::morph::MorphWeights;
    use bevy::platform::collections::HashMap;
    use bevy::prelude::*;
    use std::sync::{Arc, Mutex};

    use super::{
        VrmLoader, add_expression_bind_pass, is_vrm_path, release_binary_chunk, vrm_load_settings,
    };

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

    #[test]
    fn the_loader_claims_only_the_vrm_extension() -> TestResult {
        assert_eq!(loader().extensions(), &["vrm"]);
        success!()
    }

    /// Whatever `GltfPlugin` or the caller asks for, a `.vrm` is loaded in glTF
    /// space with its source kept, so `VrmExtensions::from_gltf` works on it.
    #[test]
    fn a_vrm_load_keeps_the_source_and_never_converts_coordinates() -> TestResult {
        let requested = GltfLoaderSettings {
            include_source: false,
            convert_coordinates: Some(GltfConvertCoordinates {
                rotate_scene_entity: true,
                rotate_meshes: true,
            }),
            load_lights: false,
            ..Default::default()
        };

        let settings = vrm_load_settings(&requested);

        assert!(settings.include_source);
        let convert = settings
            .convert_coordinates
            .expect("explicit, so `GltfPlugin::convert_coordinates` cannot apply");
        assert!(!convert.rotate_scene_entity);
        assert!(!convert.rotate_meshes);
        assert!(!settings.load_lights, "the other settings pass through");
        success!()
    }

    /// Only a `.vrm` gets the VRM settings: bevy also routes a typed
    /// `load::<Gltf>` whose extension no loader claims exactly to this loader.
    #[test]
    fn only_a_vrm_path_is_loaded_as_a_vrm() -> TestResult {
        use bevy::asset::AssetPath;
        assert!(is_vrm_path(&AssetPath::from("vrm/Elmer.vrm")));
        assert!(is_vrm_path(&AssetPath::from("vrm/Elmer.VRM")));
        assert!(is_vrm_path(&AssetPath::from("vrm/Elmer.vrm#Scene0")));
        assert!(!is_vrm_path(&AssetPath::from("models/Ship.GLB")));
        assert!(!is_vrm_path(&AssetPath::from("models/ship.gltf")));
        assert!(!is_vrm_path(&AssetPath::from("https://example.com/model")));
        success!()
    }

    /// The kept source keeps its JSON document, which is all the VRM extensions
    /// need, but not the file-sized binary chunk.
    #[test]
    fn the_kept_source_releases_its_binary_chunk() -> TestResult {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/vrm/AliciaSolid.vrm");
        let mut source = Some(::gltf::Gltf::from_slice_without_validation(
            &std::fs::read(path)?,
        )?);
        assert!(source.as_ref().is_some_and(|source| source.blob.is_some()));

        release_binary_chunk(&mut source);

        let source = source.expect("the source is kept");
        assert!(source.blob.is_none());
        assert!(
            source
                .extensions()
                .is_some_and(|extensions| extensions.contains_key("VRMC_vrm"))
        );
        success!()
    }

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
