## Unreleased

### Breaking Changes

- `MToonOutline` is no longer a component; it has become part of the `MToonMaterial` fields.
- `OutlineWidthMode` has been added as part of the field of `MToonOutline`.

### Bug Fixes

- Fixed outline rendering 

## v0.1.2

[Relsease Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.1.2)

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