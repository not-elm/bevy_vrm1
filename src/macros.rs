#[macro_export]
macro_rules! new_type {
    (
        $(#[$meta:meta])*
        name: $struct_name: ident,
        ty: String,
    ) => {
        $(#[$meta])*
        #[derive(
            bevy::prelude::Component,
            bevy::prelude::Reflect,
            Ord,
            PartialOrd,
            Eq,
            PartialEq,
            Debug,
            serde::Serialize,
            serde::Deserialize,
            Clone,
            Hash,
            bevy::prelude::Deref,
        )]
        pub struct $struct_name(pub String);

        impl From<&str> for $struct_name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl std::fmt::Display for $struct_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
    (
        $(#[$meta:meta])*
        name: $struct_name: ident,
        ty: $ty: ident,
    ) => {
        #[derive(
            bevy::prelude::Component,
            Eq,
            PartialEq,
            Debug,
            serde::Serialize,
            serde::Deserialize,
            Copy,
            Clone,
            Hash,
            bevy::prelude::Deref,
        )]
        pub struct $struct_name(pub $ty);

        impl From<$ty> for $struct_name {
            fn from(value: $ty) -> Self {
                Self(value)
            }
        }
    };
}

macro_rules! marker_component {
        (
            $(#[$meta:meta])*
            $name: ident
        ) => {
            $(#[$meta])*
            #[derive(
                Component,
                Default,
                Debug,
                Copy,
                Clone,
                Eq,
                PartialEq,
                Hash,
                Reflect,
                serde::Serialize,
                serde::Deserialize,
            )]
            #[reflect(Component, Serialize, Deserialize, Default)]
            pub struct $name;
        };
    }

macro_rules! entity_component {
        (
            $(#[$meta:meta])*
            $name: ident
        ) => {
            $(#[$meta])*
            #[derive(
                Component,
                Debug,
                Copy,
                Clone,
                Eq,
                PartialEq,
                Hash,
                Reflect,
                bevy::prelude::Deref,
            )]
            #[reflect(Component)]
            #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
            #[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
            pub struct $name(pub bevy::prelude::Entity);
        };
    }

pub(crate) use marker_component;
pub(crate) use entity_component;
