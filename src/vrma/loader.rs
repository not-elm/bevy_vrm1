//! This module provides the functionality to load VRMA files.

use bevy::app::{App, Plugin};
use bevy::asset::io::Reader;
use bevy::asset::{Asset, AssetLoader, LoadContext};
use bevy::gltf::{
    DefaultGltfImageSampler, Gltf, GltfError, GltfLoader, GltfLoaderSettings,
    extensions::GltfExtensionHandlers,
};
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats, ImageSamplerDescriptor};
use bevy::mesh::MeshCompressionArgs;
use bevy::prelude::*;
use bevy::utils::default;

pub(super) struct VrmaLoaderPlugin;

impl Plugin for VrmaLoaderPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.init_asset::<VrmaAsset>()
            .preregister_asset_loader::<VrmaLoader>(&["vrma"]);
    }

    fn finish(
        &self,
        app: &mut App,
    ) {
        let supported_compressed_formats =
            if let Some(resource) = app.world().get_resource::<CompressedImageFormatSupport>() {
                resource.0
            } else {
                CompressedImageFormats::NONE
            };
        let default_sampler =
            if let Some(resource) = app.world().get_resource::<DefaultGltfImageSampler>() {
                resource.get_internal()
            } else {
                let resource = DefaultGltfImageSampler::new(&ImageSamplerDescriptor::default());
                let sampler = resource.get_internal();
                app.insert_resource(resource);
                sampler
            };
        let extensions = if let Some(resource) = app.world().get_resource::<GltfExtensionHandlers>()
        {
            resource.0.clone()
        } else {
            let resource = GltfExtensionHandlers::default();
            let handlers = resource.0.clone();
            app.insert_resource(resource);
            handlers
        };
        app.register_asset_loader(VrmaLoader(GltfLoader {
            supported_compressed_formats,
            custom_vertex_attributes: Default::default(),
            default_sampler,
            default_convert_coordinates: Default::default(),
            extensions,
            default_skinned_mesh_bounds_policy: Default::default(),
            default_mesh_compression: MeshCompressionArgs::none(),
        }));
    }
}

/// Represents a VRMA asset.
/// You can load it using [`AssetServer`].
///
///```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
/// use bevy_vrm1::vrma::VrmaHandle;
///
/// fn spawn_vrma(
///    mut commands: Commands,
///    asset_server: Res<AssetServer>,
/// ){
///    commands.spawn(VrmaHandle(asset_server.load("<vrma>.vrma")));
/// }
/// ```
#[derive(Debug, Asset, TypePath)]
pub struct VrmaAsset {
    pub gltf: Gltf,
}

#[derive(TypePath)]
struct VrmaLoader(GltfLoader);

impl AssetLoader for VrmaLoader {
    type Asset = VrmaAsset;
    type Settings = ();
    type Error = GltfError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let settings = GltfLoaderSettings {
            include_source: true,
            ..default()
        };
        let gltf = self.0.load(reader, &settings, load_context).await?;
        Ok(VrmaAsset { gltf })
    }

    fn extensions(&self) -> &[&str] {
        &["vrma"]
    }
}
