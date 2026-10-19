# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`bevy_vrm1` is a Bevy plugin for loading and animating VRM 1.0 models and VRMA animations. It supports Spring Bone physics, LookAt gaze control, Node Constraints, and Expression systems following the official VRM specification update order.

**Important**: Only VRM 1.0 is supported. This crate is in early development and may undergo breaking changes.

## Development Commands

### Build and Check
```bash
# Check compilation
cargo check

# Build the project
cargo build

# Build with features
cargo build --features serde,log
```

### Testing
```bash
# Run all tests
cargo test

# Run a specific test
cargo test test_name

# Run tests with logging
cargo test --features log
```

### Running Examples
```bash
# Basic VRM loading
cargo run --example simple

# Spring bone physics demo
cargo run --example spring_bone

# LookAt demos
cargo run --example look_at_cursor
cargo run --example look_at_target

# VRMA animation playback
cargo run --example vrma
cargo run --example vrma_transition

# MToon multiple directional lights
cargo run --example multiple_lights
```

### Linting
The project uses Clippy with custom lints defined in `Cargo.toml`:
```bash
cargo clippy
```

## Architecture Overview

### Plugin Structure

The `VrmPlugin` is the main entry point that orchestrates sub-plugins:

```
VrmPlugin
├── VrmInstancePlugin        (VrmHandle spawning & root binding)
├── VrmSpringBonePlugin      (Spring physics)
├── VrmHumanoidBonePlugin    (Bone hierarchy mapping)
├── VrmExpressionPlugin      (Morph target expressions)
├── MtoonMaterialPlugin      (Shader & material rendering)
└── LookAtPlugin             (Gaze control system)
```

Loading `.vrm` files is a separate plugin (`VrmGltfPlugin`) that must be added after `DefaultPlugins` alongside VrmPlugin; it also adds `VrmGltfRuntimePlugin` (node constraints).
VRMA (animation) is a separate plugin (`VrmaPlugin`) that works alongside VrmPlugin.

### VRM Asset Loading Pipeline

1. **VrmHandle → Gltf**: User spawns entity with `VrmHandle`, `VrmLoader` loads the `.vrm` through Bevy's glTF loader
2. **Extensions → Components**: `GltfExtensionHandler` hooks (`src/vrm/gltf/handler/`) parse the VRM extensions and write the VRM components into the scene asset while the file loads
3. **Root Binding**: When the scene instance is ready, the root data is moved onto the `VrmHandle` entity and `Initialized` is inserted

### Critical System Execution Order (VrmSystemSets)

The system execution order follows the [VRM specification](https://vrm.dev/api/api_update/):

```
Animation (Bevy standard)
    ↓
VrmSystemSets::Constraints
    ↓
VrmSystemSets::PropagateAfterConstraints (manual transform propagation)
    ↓
VrmSystemSets::GazeControl (LookAt)
    ↓
VrmSystemSets::Expressions
    ↓
VrmSystemSets::PropagateAfterExpressions (manual transform propagation)
    ↓
VrmSystemSets::SpringBone
    ↓
VrmSystemSets::DetermineRedraw (triggers RequestRedraw if needed)
```

**Important**: Manual transform propagation is inserted at two points to ensure `GlobalTransform` is updated before downstream systems use it. This is critical for correct rendering and physics.

### Key Architectural Patterns

#### 1. Load-Time Resolution
Metadata extracted from glTF extensions is resolved into components while the file loads, so nothing is bound by name at runtime:
- `<Bone>BoneEntity` holders: Point from the VRM root to the humanoid bone entities
- `ExpressionSettings` / `ExpressionMorphBinds`: Map expression names to morph target bindings
- `VrmNodeConstraint` / `ConstraintExecutionOrder`: Bind constraint destinations to their sources

`HumanoidBoneRegistry` remains only for the VRMA source rig.

#### 2. RestTransform Baseline
Systems use stored `RestTransform`/`RestGlobalTransform` (captured at initialization) as a baseline to compute deltas. This enables multiple systems to read the same base state without conflicts.

#### 3. Entity Remapping
Components written into the scene asset are copied into each instance through reflection, so they must be registered with `register_type`, and any `Entity` field must be marked `#[entities]` (see `entity_component!` in `src/macros.rs`).

#### 4. VRMA Retargeting
VRMA maintains separate registries per skeleton and uses custom animation curves (`BoneRotationAnimationCurve`, `HipsTranslationAnimationCurve`) to retarget animations from VRMA skeleton to VRM skeleton.

## Component Constraints

### Node Constraint System

Three constraint types (evaluated in `ConstraintExecutionOrder` by one pass during `VrmSystemSets::Constraints`):

- **RotationConstraint**: Transfers entire local rotation from source to destination (use case: sub-arms)
- **RollConstraint**: Transfers rotation around a specific axis only (use case: twist bones)
- **AimConstraint**: Rotates destination to face source (use case: clothing accessories)

All use spherical linear interpolation (slerp) based on weight parameter (0.0-1.0).

### Spring Bone Physics

- Runs **after** all pose changes in `VrmSystemSets::SpringBone`
- Uses Verlet integration for physics simulation
- Each `SpringRoot` contains a chain of joints with collision detection
- Center node defines reference frame for inertia calculations

### LookAt System

Two modes:
- **Cursor Mode**: Tracks mouse cursor position via camera ray casting
- **Target Mode**: Tracks a specific entity

Updates `Head`, `LeftEye`, `RightEye` bone rotations based on `LookAtProperties` ranges.

## Transform Propagation Strategy

Bevy's default `TransformPropagate` runs once in `PostUpdate`. This crate manually invokes transform propagation **twice** to comply with VRM spec:

1. **After Constraints**: Ensures constraint changes propagate to `GlobalTransform` before LookAt reads positions
2. **After Expressions**: Ensures expression changes propagate before SpringBone physics reads positions

This is implemented in `src/vrm.rs` using:
```rust
use bevy::transform::systems::{propagate_parent_transforms, sync_simple_transforms};

app.add_systems(
    PostUpdate,
    (sync_simple_transforms, propagate_parent_transforms)
        .chain()
        .in_set(VrmSystemSets::PropagateAfterConstraints)
);
```

## Working with VRM Specifications

When modifying update order or system timing, always reference:
- [VRM Update Order Specification](https://vrm.dev/api/api_update/)
- [Spring Bone Specification](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_springBone-1.0/README.md)
- [Node Constraint Specification](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_node_constraint-1.0/README.md)
- [LookAt Specification](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/lookAt.md)
- [VRMA Specification](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm_animation-1.0/README.md)

## Version Compatibility

| bevy_vrm1 | bevy |
|-----------|------|
| 0.8.0 ~   | 0.19 |
| 0.5.0 ~   | 0.18 |
| 0.4.0 ~   | 0.17 |
| 0.1.0 ~   | 0.16 |

Rust edition: 2024

## Module Organization

```
src/
├── lib.rs                  (Main exports)
├── system_set.rs           (VrmSystemSets enum)
├── system_param.rs         (Helper system params: ChildSearcher, ParentSearcher, etc.)
├── vrm/                    (VRM 1.0 implementation)
│   ├── instance.rs         (VrmHandle spawning & root binding)
│   ├── expressions.rs      (Expression triggers)
│   ├── humanoid_bone.rs    (Bone mapping)
│   ├── look_at.rs          (Gaze control)
│   ├── spring_bone/        (Physics simulation)
│   ├── runtime.rs          (Node constraint evaluation)
│   ├── mtoon/              (Shader implementation)
│   └── gltf/               (VrmGltfPlugin: glTF extension parsing & load-time handler)
└── vrma/                   (VRMA animation implementation)
    ├── loader.rs           (VRMA asset loading)
    ├── initialize.rs       (VRMA scene setup)
    └── animation/          (Retargeting system)
```

## Testing Notes

- Tests use `bevy_test_helper` for setting up minimal Bevy apps
- Test VRM models are in `assets/` (excluded from crate publication)
- Sample model credit: **AliciaSolid** by **© DWANGO Co., Ltd.**

## Common Pitfalls

1. **System Ordering**: When adding new VRM-related systems, always ensure they run in the correct `VrmSystemSets` and respect the specification order
2. **Transform Propagation**: If a system modifies `Transform` and another system needs to read `GlobalTransform` in the same frame, manual propagation may be needed
3. **Bone Dependencies**: Systems that need bone entities should read the `<Bone>BoneEntity` holders on the VRM root, which exist once `Initialized` is inserted
