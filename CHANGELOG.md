## Unreleased

### Bug Fixes

- Fixed SpringBone colliders.

## v0.2.1

[Relsease Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.2.1)
I was going to add this in v0.2.0 but forgot.

### Improvements

- Added `VrmSystemSets` to define the system order of `Retarget`, `LookAt`, and `SpringBone`.
- Export several VRM(A) components that were not being exported correctly via `prelude` module.

## v0.2.0

[Relsease Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.2.0)

### Breaking Changes

- `MToonOutline` is no longer a component; it has become part of the `MToonMaterial` fields.
- `OutlineWidthMode` has been added as part of the field of `MToonOutline`.
    - Currently only supports `OutlineWidthMode::WorldCoordinates` and `OutlineWidthMode::None`, and if
      `screenCoordinates` is passed, the outline will not be rendered.
- Fixed the rendering order of the outline to match the spec.
    - refer to
      the [here](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#rendering)
      for more details.
- Removed `reflect` feature flag, and `serde` has been added instead.
    - `Reflect` is now applied to most structs by default.

### Bug Fixes

- Fixed outline rendering

## v0.1.2

[Relsease Notes](https:/x/github.com/not-elm/bevy_vrm1/releases/tag/v0.1.2)

### Bug Fixes

- Fixed so that retargeting bone works correctly between models with different initial poses.
- Fixed a bug that only one animation could be played.

## v0.1.1

[Relsease Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.1.1)

### Bug Fixes

- Fixed `VrmcMaterialsExtensitions::outline_width_factor` type from `f32` to `Option<f32>` to match the spec.
- Fixed shadow casting for directional lights.

### Features

- Supported multiple directional lights

## v0.1.0

First Release!