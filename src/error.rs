pub type AppResult<T = ()> = Result<T, anyhow::Error>;

macro_rules! vrm_error {
    ($err:expr) => {
        let _e = $err;
        #[cfg(feature = "log")]
        bevy::log::error!("{_e}")
    };
    ($message:literal, $err: expr) => {
        let _e = $err;
        #[cfg(feature = "log")]
        bevy::log::error!("{}: {_e}", $message)
    };
    ($($arg:tt)*) => {{
        #[cfg(feature = "log")]
        bevy::log::error!($($arg)*)
    }};
}

pub(crate) use vrm_error;

macro_rules! vrm_warn {
    ($err:expr) => {
        let _e = $err;
        #[cfg(feature = "log")]
        bevy::log::warn!("{_e}")
    };
    ($message:literal, $err: expr) => {
        let _e = $err;
        #[cfg(feature = "log")]
        bevy::log::warn!("{}: {_e}", $message)
    };
    ($($arg:tt)*) => {{
        #[cfg(feature = "log")]
        bevy::log::warn!($($arg)*)
    }};
}

pub(crate) use vrm_warn;
