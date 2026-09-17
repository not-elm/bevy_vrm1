## Unreleased

### Maintenance

- Updated the Windows cursor fallback dependency to `windows` 0.62
- Used fixed-size triangle chunks in first-person mesh classification to satisfy current Clippy

## v0.9.2

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.9.2)

### Bug Fixes

- Allowed `VRMC_springBone` to omit the optional `colliders`, `colliderGroups`, and `springs` arrays, defaulting them to empty collections as specified

## v0.9.1

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.9.1)

### Bug Fixes

- Fixed spring-bone chains disappearing when an inverse point transform rounded a very short tail direction to zero

## v0.9.0

### Features

- Added `VRMC_vrm.firstPerson` support: attach `FirstPersonCamera` or `ThirdPersonCamera` to a camera and trigger `RequestEnableFirstPerson` on the VRM entity to assign `RenderLayers` from the model's `meshAnnotations`. Meshes without an annotation are split by head bone weights, `RequestDisableFirstPerson` restores the default visibility, and the layers used for the separation are configurable through the `FirstPersonLayers` resource

## v0.8.0

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.8,0)

### Breaking Changes

- Migrates bevy_vrm1 from Bevy 0.18 to Bevy 0.19.

## v0.7.1

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.7.1)

### Bug Fixes

- Fixed MToon conversion skipping meshes whose glTF material name collided with another material: `VrmcMaterialRegistry` now resolves material handles by index instead of by name, so VRMs exported with duplicate material names (e.g. VRoid models with multiple `Body_mtoon` entries) no longer fall back to the default `StandardMaterial` for the collided meshes
- Fixed MToon characters not being lit by directional lights that have `shadows_enabled: false`: the MToon shader was using `shadows_enabled` to gate the entire light contribution, which is inconsistent with Bevy PBR where `shadows_enabled` only controls shadow-map sampling. `apply_directional_lights` now accumulates every directional light's contribution, and `calc_mtoon_lighting_shading` defaults `shadow` to `1.0` when the light has no shadow map
- Fixed full-viewport blackout on WebGPU when rendering MToon materials: `apply_emissive_light` was reading the `EMISSIVE_TEXTURE` bit from `PbrInput.flags` (the standard-material flags field, which `MToonMaterial` does not populate), causing an unbound-texture sample whose NaN/Inf output contaminated the HDR tonemap/bloom path. The shader now reads the bit from the MToon uniform `material.flags`, and `MtoonFlags::from(&MToonMaterial)` now actually sets the `EMISSIVE_TEXTURE` bit based on `emissive_texture.is_some()` so the branch works as intended

## v0.7.0

### Breaking Changes

- Redesigned VRMA retargeting: replaced custom `AnimationCurve` wrappers with pre-baked clips that apply retarget transformations at initialization time

### Bug Fixes

- Fixed multi-VRM animation stop bug: spawning 2+ VRMs from the same `.vrm` file no longer causes animations to stop
- Fixed animation transition interpolation for VRMs with different initial poses (Issue #32)

## v0.6.4

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.6.4)

### Features

- Added VRM detach request handling via `RequestDetachVrm`; detaching now removes VRM-related components and recursively despawns child entities while keeping the root entity alive

## v0.6.3

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.6.3)

### Others

- Removed unnecessary cursor position fallback log output

## v0.6.2

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.6.2)

### Bug Fixes

- Fixed outline rendering for double-sided meshes: skip outline enqueueing when neither CULL_FRONT nor CULL_BACK is set in MToonMaterialKey, preventing magenta-like artifacts on thin meshes such as skirts and sleeves

## v0.6.1

### Bug Fixes

- Fixed the issue on Windows where the cursor position couldn't be retrived correctly when hit_test is false, by switching to use WinApi.

## v0.6.0

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.6.0)

### Breaking Changes

- Simplified `LookAt::Cursor { camera: Option<Entity> }` to `LookAt::Cursor`
- `ModifyExpressions` doc comments updated to clarify its role as a partial update API (equivalent to UniVRM's `SetWeight` / three-vrm's `setValue`)
- `VrmExpressionRegistry` value type changed from `Vec<ExpressionNode>` to `ExpressionMetadata`

### Bug Fixes

- Fixed hips retargeting phantom X/Z shift by using local rest positions for delta computation instead of global positions; global positions are now only used for Y-based height scaling
- Fixed LookAt Cursor mode to use world-space ray casting instead of screen-space normalized coordinates, so gaze calculation now accounts for the avatar's world position
- Fixed `MorphTargetBind.weight` being parsed but ignored — now correctly applied as `expression_weight × bind.weight`
- Fixed expression weights using direct assignment instead of additive accumulation per VRM 1.0 spec
- Implemented `overrideBlink`/`overrideLookAt`/`overrideMouth` expression override system (was parsed but unused)
- Implemented `isBinary` threshold behavior (weight > 0.5 → 1.0, otherwise 0.0)

### Features

- Added direct expression control API (`SetExpressions`, `ClearExpressions`) for controlling VRM facial expression weights from user code without VRMA animation files
- Added `ExpressionEntityMap` component for O(1) expression entity lookups and introspection of available expressions
- Added `expressions` example demonstrating keyboard-driven expression control
- The `BodyTracking` component has been added. By inserting it with LookAt, you can control not only the eyes but also the upper body.

### Improvements

- Made Spring structs (`SpringRoot`, `SpringJoints`, `SpringJointProps`) public
- Moved `bind_expressions` system from `VrmaRetargetExpressionsPlugin` to `VrmExpressionPlugin` so expressions work with or without VRMA

## v0.5.1

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.5.1)

### Bug Fixes

- Fixed MToon outline rendering pipeline after Bevy 0.17+ migration
  - Outline pass now correctly uses MToonMaterial vertex/fragment shaders instead of default PBR
  - Added MToon material bind group layout at index 3 with MATERIAL_BIND_GROUP=3

## v0.5.0

- Migrated Bevy dependency from v0.17 to v0.18.

## v0.4.0

### Breaking Changes

- Migrated Bevy dependency from v0.16 to v0.17.
- Outline rendering now uses a strict depth compare to avoid full-surface outline fill on thin meshes.

### Bug Fixes

- Fixed system execution order for VRM constraints and expressions to comply with VRM specification
  - Added manual transform propagation after constraints and expressions
  - Ensures `GlobalTransform` updates propagate correctly between systems
  - Fixes rendering and physics issues caused by stale transform data

## v0.3.0

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.3.0)

### Features

- Added support for Node Constraints (VRMC_node_constraint-1.0)
  - Rotation Constraint: Transfers entire local rotation from source to destination nodes
  - Roll Constraint: Transfers rotation around a specific axis (X, Y, or Z)
  - Aim Constraint: Rotates a node to face a target node
  - All constraint types support weight-based interpolation using spherical linear interpolation (slerp)

### Breaking Changes

- `AnimationTransitions` are now used internally;This enables smooth animation transitions.
  - Changed fields of `PlayVrma`
- added `log` feature flag to enable logging.
  - Error logs are now not output by default.
- The update timing for SpringBone and LookAt has been changed to `PostUpdate`.
- Rust edition has been changed to 2024.
- Renamed some of the methods defined on SystemParams in this crate.
  - Doesn't affect most users

### Bug Fixes

- Fixed collision detection for the SpringBone sphere collider.
- Fixed logic to determine redraw
- Fixed look at bone rotation
- Fixed `ColliderGroup::name` types from `String` to `Option<String>` to match the spec.

## v0.2.2

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.2.2)

### Bug Fixes

- Fixed SpringBone colliders.
- Changed the spring bone calculation to use the center space if a center node is set.
- Fixed an issue that caused a crash during MToon shader processing.
  - This occurred in Bevy v0.16.1 and later versions.

## v0.2.1

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.2.1)
I was going to add this in v0.2.0 but forgot.

### Improvements

- Added `VrmSystemSets` to define the system order of `Retarget`, `LookAt`, and `SpringBone`.
- Export several VRM(A) components that were not being exported correctly via `prelude` module.

## v0.2.0

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.2.0)

### Breaking Changes

- `MToonOutline` is no longer a component; it has become part of the `MToonMaterial` fields.
- `OutlineWidthMode` has been added as part of the field of `MToonOutline`.
  - Currently only supports `OutlineWidthMode::WorldCoordinates` and `OutlineWidthMode::None`, and if
    `screenCoordinates` is passed, the outline will not be rendered.
- Fixed the rendering order of the outline to match the spec.
  - refer
    to [here](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#rendering)
    for more details.
- Removed `reflect` feature flag, and `serde` has been added instead.
  - `Reflect` is now applied to most structs by default.

### Bug Fixes

- Fixed outline rendering

## v0.1.2

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.1.2)

### Bug Fixes

- Fixed so that retargeting bone works correctly between models with different initial poses.
- Fixed a bug that only one animation could be played.

## v0.1.1

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.1.1)

### Bug Fixes

- Fixed `VrmcMaterialsExtensitions::outline_width_factor` type from `f32` to `Option<f32>` to match the spec.
- Fixed shadow casting for directional lights.

### Features

- Supported multiple directional lights

## v0.1.0

First Release!
