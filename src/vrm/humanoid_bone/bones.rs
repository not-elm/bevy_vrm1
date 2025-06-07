use crate::macros::{marker_component, entity_component};
use bevy::prelude::*;

macro_rules! bone_marker_component {
    ($name: ident) => {
        marker_component!(
            #[doc = concat!(
                "This marker component indicates that the entity is a `",
                stringify!($name),
                "` bone.\n\
                 This is automatically inserted after the VRM or VRMA has loaded."
            )]
            $name
        );

        paste::paste!{
            entity_component!(
                #[doc = concat!(
                "This component holds the entity of `",
                stringify!($name),
                "` bone.\n\
                 This is automatically inserted into the entity of VRM or VRMA, after they have finished loading."
                )]
                [<$name BoneEntity>]
            );
        }
    };

    ($($names: ident),+ $(,)?) => {
        $(bone_marker_component!($names);)+
    };
}

bone_marker_component!(
    Hips,
    RightRingProximal,
    RightThumbDistal,
    RightRingIntermediate,
    RightUpperArm,
    LeftIndexProximal,
    LeftUpperLeg,
    LeftFoot,
    LeftIndexDistal,
    LeftThumbMetacarpal,
    RightLowerArm,
    LeftMiddleDistal,
    RightUpperLeg,
    LeftToes,
    LeftThumbDistal,
    RightShoulder,
    RightThumbMetacarpal,
    Spine,
    LeftLowerLeg,
    LeftShoulder,
    LeftUpperArm,
    UpperChest,
    RightToes,
    RightIndexDistal,
    LeftMiddleProximal,
    LeftRingProximal,
    LeftRingDistal,
    LeftThumbProximal,
    LeftIndexIntermediate,
    LeftLittleProximal,
    LeftLittleDistal,
    RightHand,
    RightLittleProximal,
    LeftRingIntermediate,
    RightIndexIntermediate,
    Chest,
    LeftHand,
    RightLittleIntermediate,
    RightFoot,
    RightLowerLeg,
    LeftLittleIntermediate,
    LeftLowerArm,
    RightLittleDistal,
    RightMiddleIntermediate,
    RightMiddleProximal,
    RightThumbProximal,
    Neck,
    Jaw,
    Head,
    LeftEye,
    RightEye,
    LeftMiddleIntermediate,
    RightRingDistal,
    RightIndexProximal,
    RightMiddleDistal,
);

pub(super) struct BonesPlugin;

impl Plugin for BonesPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<Hips>()
            .register_type::<RightRingProximal>()
            .register_type::<RightThumbDistal>()
            .register_type::<RightRingIntermediate>()
            .register_type::<RightUpperArm>()
            .register_type::<LeftIndexProximal>()
            .register_type::<LeftUpperLeg>()
            .register_type::<LeftFoot>()
            .register_type::<LeftIndexDistal>()
            .register_type::<LeftThumbMetacarpal>()
            .register_type::<RightLowerArm>()
            .register_type::<LeftMiddleDistal>()
            .register_type::<RightUpperLeg>()
            .register_type::<LeftToes>()
            .register_type::<LeftThumbDistal>()
            .register_type::<RightShoulder>()
            .register_type::<RightThumbMetacarpal>()
            .register_type::<Spine>()
            .register_type::<LeftLowerLeg>()
            .register_type::<LeftShoulder>()
            .register_type::<LeftUpperArm>()
            .register_type::<UpperChest>()
            .register_type::<RightToes>()
            .register_type::<RightIndexDistal>()
            .register_type::<LeftMiddleProximal>()
            .register_type::<LeftRingProximal>()
            .register_type::<LeftRingDistal>()
            .register_type::<LeftThumbProximal>()
            .register_type::<LeftIndexIntermediate>()
            .register_type::<LeftLittleProximal>()
            .register_type::<LeftLittleDistal>()
            .register_type::<RightHand>()
            .register_type::<RightLittleProximal>()
            .register_type::<LeftRingIntermediate>()
            .register_type::<RightIndexIntermediate>()
            .register_type::<Chest>()
            .register_type::<LeftHand>()
            .register_type::<RightLittleIntermediate>()
            .register_type::<RightFoot>()
            .register_type::<RightLowerLeg>()
            .register_type::<LeftLittleIntermediate>()
            .register_type::<LeftLowerArm>()
            .register_type::<RightLittleDistal>()
            .register_type::<RightMiddleIntermediate>()
            .register_type::<RightMiddleProximal>()
            .register_type::<RightThumbProximal>()
            .register_type::<Neck>()
            .register_type::<Jaw>()
            .register_type::<Head>()
            .register_type::<LeftEye>()
            .register_type::<RightEye>()
            .register_type::<LeftMiddleIntermediate>()
            .register_type::<RightRingDistal>()
            .register_type::<RightIndexProximal>()
            .register_type::<RightMiddleDistal>()
            .register_type::<HipsBoneEntity>()
            .register_type::<RightRingProximalBoneEntity>()
            .register_type::<RightThumbDistalBoneEntity>()
            .register_type::<RightRingIntermediateBoneEntity>()
            .register_type::<RightUpperArmBoneEntity>()
            .register_type::<LeftIndexProximalBoneEntity>()
            .register_type::<LeftUpperLegBoneEntity>()
            .register_type::<LeftFootBoneEntity>()
            .register_type::<LeftIndexDistalBoneEntity>()
            .register_type::<LeftThumbMetacarpalBoneEntity>()
            .register_type::<RightLowerArmBoneEntity>()
            .register_type::<LeftMiddleDistalBoneEntity>()
            .register_type::<RightUpperLegBoneEntity>()
            .register_type::<LeftToesBoneEntity>()
            .register_type::<LeftThumbDistalBoneEntity>()
            .register_type::<RightShoulderBoneEntity>()
            .register_type::<RightThumbMetacarpalBoneEntity>()
            .register_type::<SpineBoneEntity>()
            .register_type::<LeftLowerLegBoneEntity>()
            .register_type::<LeftShoulderBoneEntity>()
            .register_type::<LeftUpperArmBoneEntity>()
            .register_type::<UpperChestBoneEntity>()
            .register_type::<RightToesBoneEntity>()
            .register_type::<RightIndexDistalBoneEntity>()
            .register_type::<LeftMiddleProximalBoneEntity>()
            .register_type::<LeftRingProximalBoneEntity>()
            .register_type::<LeftRingDistalBoneEntity>()
            .register_type::<LeftThumbProximalBoneEntity>()
            .register_type::<LeftIndexIntermediateBoneEntity>()
            .register_type::<LeftLittleProximalBoneEntity>()
            .register_type::<LeftLittleDistalBoneEntity>()
            .register_type::<RightHandBoneEntity>()
            .register_type::<RightLittleProximalBoneEntity>()
            .register_type::<LeftRingIntermediateBoneEntity>()
            .register_type::<RightIndexIntermediateBoneEntity>()
            .register_type::<ChestBoneEntity>()
            .register_type::<LeftHandBoneEntity>()
            .register_type::<RightLittleIntermediateBoneEntity>()
            .register_type::<RightFootBoneEntity>()
            .register_type::<RightLowerLegBoneEntity>()
            .register_type::<LeftLittleIntermediateBoneEntity>()
            .register_type::<LeftLowerArmBoneEntity>()
            .register_type::<RightLittleDistalBoneEntity>()
            .register_type::<RightMiddleIntermediateBoneEntity>()
            .register_type::<RightMiddleProximalBoneEntity>()
            .register_type::<RightThumbProximalBoneEntity>()
            .register_type::<NeckBoneEntity>()
            .register_type::<JawBoneEntity>()
            .register_type::<HeadBoneEntity>()
            .register_type::<LeftEyeBoneEntity>()
            .register_type::<RightEyeBoneEntity>()
            .register_type::<LeftMiddleIntermediateBoneEntity>()
            .register_type::<RightRingDistalBoneEntity>()
            .register_type::<RightIndexProximalBoneEntity>()
            .register_type::<RightMiddleDistalBoneEntity>();
    }
}
