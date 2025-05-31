#![allow(clippy::type_complexity, clippy::too_many_arguments)]

mod error;
mod macros;
pub mod system_param;
pub mod vrm;
pub mod vrma;

pub mod prelude{
    pub use crate::{
        vrm::*,
        vrm::{
            loader::VrmHandle,
            look_at::LookAt,
        },
    };
}

#[cfg(test)]
pub(crate) mod tests {
    use bevy::asset::AssetPlugin;
    use bevy::prelude::ImagePlugin;
    use bevy::render::camera::CameraPlugin;
    use bevy::window::WindowPlugin;
    use bevy::MinimalPlugins;

    pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    #[macro_export]
    macro_rules! success {
        () => {
            std::result::Result::Ok(())
        };
    }

    pub fn test_app() -> bevy::app::App {
        let mut app = bevy::app::App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            ImagePlugin::default(),
            WindowPlugin::default(),
            CameraPlugin,
        ));
        app
    }
}
