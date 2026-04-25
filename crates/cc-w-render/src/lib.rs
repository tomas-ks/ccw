use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};
use cc_w_types::{
    Bounds3, DefaultRenderClass, GeometryDefinitionId, GeometryInstanceId, PickHit, PickRegion,
    PickResult, PreparedMaterial, PreparedMesh, PreparedRenderDefinition, PreparedRenderInstance,
    PreparedRenderScene, SemanticElementId, WORLD_FORWARD, WORLD_RIGHT, WORLD_UP,
};
use glam::{DMat4, DVec3, DVec4, Mat4, Vec3};
use std::collections::{HashMap, HashSet};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;
use wgpu::util::DeviceExt;
use wgpu::vertex_attr_array;

mod mesh_edges;
mod profile;

pub use mesh_edges::{ExtractedMeshEdges, MeshEdgeExtractionConfig};
pub use profile::{
    RenderProfileDescriptor, RenderProfileId, UnknownRenderProfile, available_render_profiles,
};

pub const DEFAULT_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const ARCHITECTURAL_EDGE_WIDTH_PX: f32 = 3.5;
const ARCHITECTURAL_CREASE_ANGLE_DEGREES: f32 = 30.0;
const SCREEN_SPACE_OUTLINE_DEPTH_THRESHOLD: f32 = 0.004;
const SCREEN_SPACE_OBJECT_ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Uint;
const SCREEN_SPACE_NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub const BASIC_MESH_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
};

struct Lighting {
    light_direction : vec4<f32>,
    factors : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

@group(0) @binding(1)
var<uniform> lighting : Lighting;

struct VertexInput {
    @location(0) position : vec3<f32>,
    @location(1) normal : vec3<f32>,
    @location(2) model_col0 : vec4<f32>,
    @location(3) model_col1 : vec4<f32>,
    @location(4) model_col2 : vec4<f32>,
    @location(5) model_col3 : vec4<f32>,
    @location(6) material_color : vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) normal : vec3<f32>,
    @location(1) material_color : vec4<f32>,
};

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    var out : VertexOutput;
    let model_from_object = mat4x4<f32>(
        input.model_col0,
        input.model_col1,
        input.model_col2,
        input.model_col3,
    );
    let world_position = model_from_object * vec4<f32>(input.position, 1.0);
    out.position = camera.clip_from_world * world_position;
    out.normal = (model_from_object * vec4<f32>(input.normal, 0.0)).xyz;
    out.material_color = input.material_color;
    return out;
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    let diffuse = max(dot(normalize(input.normal), normalize(lighting.light_direction.xyz)), 0.0);
    let lit = lighting.factors.x + (diffuse * lighting.factors.y);
    return vec4<f32>(input.material_color.xyz * lit, input.material_color.w);
}
"#;

pub const ARCHITECTURAL_MESH_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
};

struct Lighting {
    light_direction : vec4<f32>,
    factors : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

@group(0) @binding(1)
var<uniform> lighting : Lighting;

struct VertexInput {
    @location(0) position : vec3<f32>,
    @location(1) normal : vec3<f32>,
    @location(2) model_col0 : vec4<f32>,
    @location(3) model_col1 : vec4<f32>,
    @location(4) model_col2 : vec4<f32>,
    @location(5) model_col3 : vec4<f32>,
    @location(6) material_color : vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) normal : vec3<f32>,
    @location(1) material_color : vec4<f32>,
};

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    var out : VertexOutput;
    let model_from_object = mat4x4<f32>(
        input.model_col0,
        input.model_col1,
        input.model_col2,
        input.model_col3,
    );
    let world_position = model_from_object * vec4<f32>(input.position, 1.0);
    out.position = camera.clip_from_world * world_position;
    out.normal = (model_from_object * vec4<f32>(input.normal, 0.0)).xyz;
    out.material_color = input.material_color;
    return out;
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    let diffuse = max(dot(normalize(input.normal), normalize(lighting.light_direction.xyz)), 0.0);
    let ambient_fill = max(lighting.factors.x, 0.46);
    let diffuse_fill = min(lighting.factors.y, 1.0 - ambient_fill);
    let lit = ambient_fill + (diffuse * diffuse_fill);
    return vec4<f32>(input.material_color.xyz * lit, input.material_color.w);
}
"#;

pub const PICK_MESH_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

struct VertexInput {
    @location(0) position : vec3<f32>,
    @location(2) model_col0 : vec4<f32>,
    @location(3) model_col1 : vec4<f32>,
    @location(4) model_col2 : vec4<f32>,
    @location(5) model_col3 : vec4<f32>,
    @location(7) pick_index : u32,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) @interpolate(flat) pick_color : vec4<u32>,
};

fn encode_pick_index(index : u32) -> vec4<u32> {
    return vec4<u32>(
        index & 0xffu,
        (index >> 8u) & 0xffu,
        (index >> 16u) & 0xffu,
        (index >> 24u) & 0xffu,
    );
}

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    var out : VertexOutput;
    let model_from_object = mat4x4<f32>(
        input.model_col0,
        input.model_col1,
        input.model_col2,
        input.model_col3,
    );
    let world_position = model_from_object * vec4<f32>(input.position, 1.0);
    out.position = camera.clip_from_world * world_position;
    out.pick_color = encode_pick_index(input.pick_index);
    return out;
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<u32> {
    return input.pick_color;
}
"#;

pub const OBJECT_ID_MESH_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

struct VertexInput {
    @location(0) position : vec3<f32>,
    @location(2) model_col0 : vec4<f32>,
    @location(3) model_col1 : vec4<f32>,
    @location(4) model_col2 : vec4<f32>,
    @location(5) model_col3 : vec4<f32>,
    @location(12) outline_index : u32,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) @interpolate(flat) object_color : vec4<u32>,
};

fn encode_object_index(index : u32) -> vec4<u32> {
    return vec4<u32>(
        index & 0xffu,
        (index >> 8u) & 0xffu,
        (index >> 16u) & 0xffu,
        (index >> 24u) & 0xffu,
    );
}

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    var out : VertexOutput;
    let model_from_object = mat4x4<f32>(
        input.model_col0,
        input.model_col1,
        input.model_col2,
        input.model_col3,
    );
    let world_position = model_from_object * vec4<f32>(input.position, 1.0);
    out.position = camera.clip_from_world * world_position;
    out.object_color = encode_object_index(input.outline_index);
    return out;
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<u32> {
    return input.object_color;
}
"#;

pub const NORMAL_MESH_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
    viewport_and_profile : vec4<f32>,
    view_from_world : mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

struct VertexInput {
    @location(0) position : vec3<f32>,
    @location(1) normal : vec3<f32>,
    @location(2) model_col0 : vec4<f32>,
    @location(3) model_col1 : vec4<f32>,
    @location(4) model_col2 : vec4<f32>,
    @location(5) model_col3 : vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) normal : vec3<f32>,
};

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    var out : VertexOutput;
    let model_from_object = mat4x4<f32>(
        input.model_col0,
        input.model_col1,
        input.model_col2,
        input.model_col3,
    );
    let world_position = model_from_object * vec4<f32>(input.position, 1.0);
    let world_normal = model_from_object * vec4<f32>(input.normal, 0.0);
    out.position = camera.clip_from_world * world_position;
    out.normal = normalize((camera.view_from_world * world_normal).xyz);
    return out;
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(input.normal);
    return vec4<f32>((normal * 0.5) + vec3<f32>(0.5), 1.0);
}
"#;

pub const EDGE_RIBBON_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
    viewport_and_profile : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

struct VertexInput {
    @location(0) edge_start : vec3<f32>,
    @location(1) edge_end : vec3<f32>,
    @location(2) model_col0 : vec4<f32>,
    @location(3) model_col1 : vec4<f32>,
    @location(4) model_col2 : vec4<f32>,
    @location(5) model_col3 : vec4<f32>,
    @location(8) corner : vec2<f32>,
    @location(9) edge_kind : f32,
    @location(10) boundary_edge_visibility : f32,
    @location(11) crease_edge_visibility : f32,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) edge_side : f32,
    @location(1) edge_visibility : f32,
};

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    var out : VertexOutput;
    let model_from_object = mat4x4<f32>(
        input.model_col0,
        input.model_col1,
        input.model_col2,
        input.model_col3,
    );
    let start_clip = camera.clip_from_world * (model_from_object * vec4<f32>(input.edge_start, 1.0));
    let end_clip = camera.clip_from_world * (model_from_object * vec4<f32>(input.edge_end, 1.0));
    let start_ndc = start_clip.xy / start_clip.w;
    let end_ndc = end_clip.xy / end_clip.w;
    let viewport = max(camera.viewport_and_profile.xy, vec2<f32>(1.0, 1.0));
    let edge_screen = (end_ndc - start_ndc) * viewport;
    var normal_screen = vec2<f32>(-edge_screen.y, edge_screen.x);
    if (dot(normal_screen, normal_screen) < 0.0001) {
        normal_screen = vec2<f32>(0.0, 1.0);
    } else {
        normal_screen = normalize(normal_screen);
    }

    let half_width_px = camera.viewport_and_profile.z;
    let offset_ndc = (normal_screen * input.corner.y * half_width_px / viewport) * 2.0;
    let base_clip = select(start_clip, end_clip, input.corner.x > 0.5);
    out.position = vec4<f32>(
        base_clip.x + offset_ndc.x * base_clip.w,
        base_clip.y + offset_ndc.y * base_clip.w,
        base_clip.z,
        base_clip.w,
    );
    out.edge_side = input.corner.y;
    out.edge_visibility = select(
        input.boundary_edge_visibility,
        input.crease_edge_visibility,
        input.edge_kind > 0.5,
    );
    return out;
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    let half_width_px = max(camera.viewport_and_profile.z, 0.5);
    let antialias_width = min(0.95, 1.0 / half_width_px);
    let edge_alpha = 1.0 - smoothstep(1.0 - antialias_width, 1.0, abs(input.edge_side));
    return vec4<f32>(0.055, 0.075, 0.105, 0.82 * edge_alpha * input.edge_visibility);
}
"#;

pub const CREASE_RIBBON_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
    viewport_and_profile : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

struct VertexInput {
    @location(0) edge_start : vec3<f32>,
    @location(1) edge_end : vec3<f32>,
    @location(2) model_col0 : vec4<f32>,
    @location(3) model_col1 : vec4<f32>,
    @location(4) model_col2 : vec4<f32>,
    @location(5) model_col3 : vec4<f32>,
    @location(8) corner : vec2<f32>,
    @location(9) edge_kind : f32,
    @location(11) crease_edge_visibility : f32,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) edge_side : f32,
    @location(1) edge_visibility : f32,
    @location(2) edge_kind : f32,
};

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    var out : VertexOutput;
    let model_from_object = mat4x4<f32>(
        input.model_col0,
        input.model_col1,
        input.model_col2,
        input.model_col3,
    );
    let start_clip = camera.clip_from_world * (model_from_object * vec4<f32>(input.edge_start, 1.0));
    let end_clip = camera.clip_from_world * (model_from_object * vec4<f32>(input.edge_end, 1.0));
    let start_ndc = start_clip.xy / start_clip.w;
    let end_ndc = end_clip.xy / end_clip.w;
    let viewport = max(camera.viewport_and_profile.xy, vec2<f32>(1.0, 1.0));
    let edge_screen = (end_ndc - start_ndc) * viewport;
    var normal_screen = vec2<f32>(-edge_screen.y, edge_screen.x);
    if (dot(normal_screen, normal_screen) < 0.0001) {
        normal_screen = vec2<f32>(0.0, 1.0);
    } else {
        normal_screen = normalize(normal_screen);
    }

    let half_width_px = max(camera.viewport_and_profile.z * 0.72, 0.9);
    let offset_ndc = (normal_screen * input.corner.y * half_width_px / viewport) * 2.0;
    let base_clip = select(start_clip, end_clip, input.corner.x > 0.5);
    out.position = vec4<f32>(
        base_clip.x + offset_ndc.x * base_clip.w,
        base_clip.y + offset_ndc.y * base_clip.w,
        base_clip.z,
        base_clip.w,
    );
    out.edge_side = input.corner.y;
    out.edge_visibility = input.crease_edge_visibility;
    out.edge_kind = input.edge_kind;
    return out;
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    if (input.edge_kind < 0.5 || input.edge_visibility <= 0.01) {
        discard;
    }

    let half_width_px = max(camera.viewport_and_profile.z * 0.72, 0.9);
    let antialias_width = min(0.95, 1.0 / half_width_px);
    let edge_alpha = 1.0 - smoothstep(1.0 - antialias_width, 1.0, abs(input.edge_side));
    return vec4<f32>(0.045, 0.060, 0.082, 0.46 * edge_alpha * input.edge_visibility);
}
"#;

pub const SCREEN_SPACE_OUTLINE_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
    viewport_and_profile : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

@group(0) @binding(1)
var depth_texture : texture_depth_2d;

@group(0) @binding(2)
var object_id_texture : texture_2d<u32>;

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index : u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    var out : VertexOutput;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    return out;
}

fn clamped_coord(coord : vec2<i32>) -> vec2<i32> {
    let size = vec2<i32>(
        max(i32(camera.viewport_and_profile.x), 1),
        max(i32(camera.viewport_and_profile.y), 1),
    );
    return clamp(coord, vec2<i32>(0, 0), size - vec2<i32>(1, 1));
}

fn load_depth(coord : vec2<i32>) -> f32 {
    return textureLoad(depth_texture, clamped_coord(coord), 0);
}

fn decode_object_id(pixel : vec4<u32>) -> u32 {
    return pixel.x | (pixel.y << 8u) | (pixel.z << 16u) | (pixel.w << 24u);
}

fn load_object_id(coord : vec2<i32>) -> u32 {
    return decode_object_id(textureLoad(object_id_texture, clamped_coord(coord), 0));
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(floor(input.position.xy));
    let center_depth = load_depth(coord);
    let center_id = load_object_id(coord);
    let depth_threshold = max(camera.viewport_and_profile.w, 0.0001);
    var depth_edge = 0.0;
    var object_edge = 0.0;
    let offsets = array<vec2<i32>, 4>(
        vec2<i32>(1, 0),
        vec2<i32>(-1, 0),
        vec2<i32>(0, 1),
        vec2<i32>(0, -1),
    );

    for (var i = 0u; i < 4u; i = i + 1u) {
        let neighbor_coord = coord + offsets[i];
        let neighbor_depth = load_depth(neighbor_coord);
        let neighbor_id = load_object_id(neighbor_coord);
        if ((center_id != neighbor_id) && ((center_id != 0u) || (neighbor_id != 0u))) {
            object_edge = 1.0;
        }
        if ((max(center_depth, neighbor_depth) > 0.000001) &&
            (abs(center_depth - neighbor_depth) > depth_threshold)) {
            depth_edge = 1.0;
        }
    }

    let alpha = max(object_edge * 0.74, depth_edge * 0.52);
    if (alpha <= 0.01) {
        discard;
    }
    return vec4<f32>(0.035, 0.052, 0.075, alpha);
}
"#;

pub const SCREEN_SPACE_AO_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
    viewport_and_profile : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

@group(0) @binding(1)
var depth_texture : texture_depth_2d;

@group(0) @binding(2)
var normal_texture : texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index : u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    var out : VertexOutput;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    return out;
}

fn clamped_coord(coord : vec2<i32>) -> vec2<i32> {
    let size = vec2<i32>(
        max(i32(camera.viewport_and_profile.x), 1),
        max(i32(camera.viewport_and_profile.y), 1),
    );
    return clamp(coord, vec2<i32>(0, 0), size - vec2<i32>(1, 1));
}

fn load_depth(coord : vec2<i32>) -> f32 {
    return textureLoad(depth_texture, clamped_coord(coord), 0);
}

fn load_normal(coord : vec2<i32>) -> vec3<f32> {
    let encoded = textureLoad(normal_texture, clamped_coord(coord), 0).xyz;
    let normal = (encoded * 2.0) - vec3<f32>(1.0);
    return normalize(normal);
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(floor(input.position.xy));
    let center_depth = load_depth(coord);
    if (center_depth <= 0.000001) {
        discard;
    }

    let center_normal = load_normal(coord);
    let offsets = array<vec2<i32>, 12>(
        vec2<i32>(2, 0),
        vec2<i32>(-2, 0),
        vec2<i32>(0, 2),
        vec2<i32>(0, -2),
        vec2<i32>(3, 3),
        vec2<i32>(-3, 3),
        vec2<i32>(3, -3),
        vec2<i32>(-3, -3),
        vec2<i32>(7, 0),
        vec2<i32>(-7, 0),
        vec2<i32>(0, 7),
        vec2<i32>(0, -7),
    );
    var occlusion = 0.0;

    for (var i = 0u; i < 12u; i = i + 1u) {
        let sample_coord = coord + offsets[i];
        let sample_depth = load_depth(sample_coord);
        if (sample_depth <= 0.000001) {
            continue;
        }

        // Reverse-Z: larger depth values are closer to the camera.
        let depth_delta = sample_depth - center_depth;
        let absolute_depth_delta = abs(depth_delta);
        let connected_surface = 1.0 - smoothstep(0.0025, 0.0120, absolute_depth_delta);
        if (connected_surface <= 0.001) {
            continue;
        }

        let close_occluder = smoothstep(0.00002, 0.0022, depth_delta);
        let range_fade = 1.0 - smoothstep(0.0030, 0.0140, absolute_depth_delta);
        let sample_normal = load_normal(sample_coord);
        let normal_difference = 1.0 - clamp(dot(center_normal, sample_normal), 0.0, 1.0);
        let normal_weight = 0.22 + (normal_difference * 0.64);
        let contact_occlusion = close_occluder * range_fade * connected_surface * normal_weight;
        let crease_occlusion = smoothstep(0.18, 0.72, normal_difference)
            * (1.0 - smoothstep(0.0008, 0.0060, absolute_depth_delta))
            * connected_surface
            * 0.30;
        occlusion = occlusion + max(contact_occlusion, crease_occlusion);
    }

    let ao = clamp(occlusion / 12.0, 0.0, 1.0);
    let alpha = pow(ao, 0.82) * 0.52;
    if (alpha <= 0.003) {
        discard;
    }
    return vec4<f32>(0.0, 0.0, 0.0, alpha);
}
"#;

pub const REFERENCE_GRID_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

struct VertexInput {
    @location(0) position : vec3<f32>,
    @location(1) alpha : f32,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) alpha : f32,
};

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    var out : VertexOutput;
    out.position = camera.clip_from_world * vec4<f32>(input.position, 1.0);
    out.alpha = input.alpha;
    return out;
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(0.36, 0.43, 0.52, input.alpha);
}
"#;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalLight {
    pub direction: Vec3,
    pub ambient: f32,
    pub diffuse_intensity: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            direction: Vec3::new(0.35, -0.45, 0.82),
            ambient: 0.2,
            diffuse_intensity: 0.8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderDefaults {
    pub clear_color: wgpu::Color,
    pub depth_format: wgpu::TextureFormat,
    pub depth_clear_value: f32,
    pub depth_write_enabled: bool,
    pub depth_compare: wgpu::CompareFunction,
    pub front_face: wgpu::FrontFace,
    pub cull_mode: Option<wgpu::Face>,
    pub directional_light: DirectionalLight,
}

impl Default for RenderDefaults {
    fn default() -> Self {
        Self {
            clear_color: wgpu::Color {
                r: 0.04,
                g: 0.05,
                b: 0.08,
                a: 1.0,
            },
            depth_format: DEFAULT_DEPTH_FORMAT,
            depth_clear_value: 0.0,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Greater,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            directional_light: DirectionalLight::default(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct GpuReferenceGridVertex {
    position: [f32; 3],
    alpha: f32,
}

impl GpuReferenceGridVertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
            vertex_attr_array![0 => Float32x3, 1 => Float32];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuReferenceGridVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

impl GpuVertex {
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 2] =
            vertex_attr_array![0 => Float32x3, 1 => Float32x3];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct GpuEdgeVertex {
    start_position: [f32; 3],
    end_position: [f32; 3],
    corner: [f32; 2],
    edge_kind: f32,
}

impl GpuEdgeVertex {
    fn new(
        start_position: [f32; 3],
        end_position: [f32; 3],
        corner: [f32; 2],
        edge_kind: f32,
    ) -> Self {
        Self {
            start_position,
            end_position,
            corner,
            edge_kind,
        }
    }

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
            vertex_attr_array![0 => Float32x3, 1 => Float32x3, 8 => Float32x2, 9 => Float32];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuEdgeVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadedMesh {
    pub mesh_id: u64,
    pub vertex_count: usize,
    pub index_count: usize,
    pub vertex_stride: u64,
    pub shader_entry: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportSize {
    pub width: u32,
    pub height: u32,
}

impl ViewportSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn clamped(self) -> Self {
        Self {
            width: self.width.max(1),
            height: self.height.max(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: DVec3,
    pub target: DVec3,
    pub up: DVec3,
    pub vertical_fov_degrees: f64,
    pub near_plane: f64,
    pub far_plane: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: (WORLD_RIGHT * 2.5) - (WORLD_FORWARD * 4.0) + (WORLD_UP * 3.25),
            target: DVec3::ZERO,
            up: WORLD_UP,
            vertical_fov_degrees: 45.0,
            near_plane: 0.1,
            far_plane: 100.0,
        }
    }
}

impl Camera {
    pub fn view_from_world(&self) -> DMat4 {
        DMat4::look_at_rh(self.eye, self.target, resolved_up_vector(self))
    }

    pub fn clip_from_world(&self, viewport: ViewportSize) -> DMat4 {
        let viewport = viewport.clamped();
        let aspect = viewport.width as f64 / viewport.height as f64;
        let projection = DMat4::perspective_rh(
            self.vertical_fov_degrees.to_radians(),
            aspect,
            self.far_plane,
            self.near_plane,
        );

        projection * self.view_from_world()
    }

    pub fn clip_from_world_f32(&self, viewport: ViewportSize) -> Mat4 {
        mat4_from_dmat4(self.clip_from_world(viewport))
    }
}

pub fn fit_camera_to_mesh(mesh: &PreparedMesh) -> Camera {
    fit_camera_to_bounds(mesh.bounds)
}

pub fn fit_camera_to_bounds(bounds: Bounds3) -> Camera {
    fit_camera_to_min_max(bounds.min, bounds.max)
}

pub fn fit_camera_to_render_scene(scene: &PreparedRenderScene) -> Camera {
    fit_camera_to_bounds(scene.bounds)
}

pub fn pick_prepared_scene_cpu(
    scene: &PreparedRenderScene,
    camera: Camera,
    viewport: ViewportSize,
    region: PickRegion,
) -> PickResult {
    let Some(region) = clamp_pick_region(region, viewport) else {
        return PickResult::empty(region);
    };
    let viewport = viewport.clamped();
    let definitions = scene
        .definitions
        .iter()
        .map(|definition| (definition.id, definition))
        .collect::<HashMap<_, _>>();
    let clip_from_world = camera.clip_from_world(viewport);

    if region.width == 1 && region.height == 1 {
        return pick_prepared_scene_point_cpu(
            scene,
            &definitions,
            camera,
            viewport,
            region,
            clip_from_world,
        );
    }

    pick_prepared_scene_rect_cpu(
        scene,
        &definitions,
        camera,
        viewport,
        region,
        clip_from_world,
    )
}

fn pick_prepared_scene_point_cpu(
    scene: &PreparedRenderScene,
    definitions: &HashMap<GeometryDefinitionId, &PreparedRenderDefinition>,
    camera: Camera,
    viewport: ViewportSize,
    region: PickRegion,
    clip_from_world: DMat4,
) -> PickResult {
    let Some(ray) = pick_ray_for_pixel(camera, viewport, region.x, region.y) else {
        return PickResult::empty(region);
    };
    let mut best_hit = None::<(f64, PickHit)>;

    for instance in &scene.instances {
        let Some(definition) = definitions.get(&instance.definition_id) else {
            continue;
        };
        let model_from_object =
            instance.model_from_object * DMat4::from_translation(definition.mesh.local_origin);
        let centroid = instance.world_bounds.center();

        for triangle in definition.mesh.indices.chunks_exact(3) {
            let Some([a, b, c]) =
                triangle_world_points(&definition.mesh, model_from_object, triangle)
            else {
                continue;
            };
            if !triangle_may_project_to_region(clip_from_world, viewport, region, [a, b, c]) {
                continue;
            }
            let Some(distance) = intersect_ray_triangle(ray.origin, ray.direction, a, b, c) else {
                continue;
            };
            if best_hit
                .as_ref()
                .is_some_and(|(best_distance, _)| distance >= *best_distance)
            {
                continue;
            }
            best_hit = Some((
                distance,
                PickHit {
                    instance_id: instance.id,
                    element_id: instance.element_id.clone(),
                    definition_id: instance.definition_id,
                    world_centroid: centroid,
                    world_anchor: ray.origin + ray.direction * distance,
                },
            ));
        }
    }

    PickResult {
        region,
        hits: best_hit.map(|(_, hit)| vec![hit]).unwrap_or_default(),
    }
}

fn pick_prepared_scene_rect_cpu(
    scene: &PreparedRenderScene,
    definitions: &HashMap<GeometryDefinitionId, &PreparedRenderDefinition>,
    camera: Camera,
    viewport: ViewportSize,
    region: PickRegion,
    clip_from_world: DMat4,
) -> PickResult {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    for instance in &scene.instances {
        let Some(definition) = definitions.get(&instance.definition_id) else {
            continue;
        };
        let model_from_object =
            instance.model_from_object * DMat4::from_translation(definition.mesh.local_origin);

        let mut intersects_region = false;
        for triangle in definition.mesh.indices.chunks_exact(3) {
            let Some(points) = triangle_world_points(&definition.mesh, model_from_object, triangle)
            else {
                continue;
            };
            if triangle_may_project_to_region(clip_from_world, viewport, region, points) {
                intersects_region = true;
                break;
            }
        }

        if !intersects_region || !seen.insert(instance.id) {
            continue;
        }

        let centroid = instance.world_bounds.center();
        candidates.push((
            centroid.distance(camera.eye),
            PickHit {
                instance_id: instance.id,
                element_id: instance.element_id.clone(),
                definition_id: instance.definition_id,
                world_centroid: centroid,
                world_anchor: centroid,
            },
        ));
    }

    candidates.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.instance_id.0.cmp(&right.1.instance_id.0))
    });

    PickResult {
        region,
        hits: candidates.into_iter().map(|(_, hit)| hit).collect(),
    }
}

#[derive(Clone, Copy, Debug)]
struct PickRay {
    origin: DVec3,
    direction: DVec3,
}

fn pick_ray_for_pixel(
    camera: Camera,
    viewport: ViewportSize,
    pixel_x: u32,
    pixel_y: u32,
) -> Option<PickRay> {
    let viewport = viewport.clamped();
    let ndc_x = ((f64::from(pixel_x) + 0.5) / f64::from(viewport.width)) * 2.0 - 1.0;
    let ndc_y = 1.0 - (((f64::from(pixel_y) + 0.5) / f64::from(viewport.height)) * 2.0);
    let world_from_clip = camera.clip_from_world(viewport).inverse();
    let near = unproject_clip_point(world_from_clip, DVec4::new(ndc_x, ndc_y, 1.0, 1.0))?;
    let far = unproject_clip_point(world_from_clip, DVec4::new(ndc_x, ndc_y, 0.0, 1.0))?;
    let direction = (far - near).try_normalize()?;
    Some(PickRay {
        origin: near,
        direction,
    })
}

fn unproject_clip_point(world_from_clip: DMat4, clip: DVec4) -> Option<DVec3> {
    let world = world_from_clip * clip;
    if world.w.abs() <= f64::EPSILON {
        return None;
    }
    Some(world.truncate() / world.w)
}

fn triangle_world_points(
    mesh: &PreparedMesh,
    model_from_object: DMat4,
    triangle: &[u32],
) -> Option<[DVec3; 3]> {
    let a = mesh.vertices.get(*triangle.first()? as usize)?;
    let b = mesh.vertices.get(*triangle.get(1)? as usize)?;
    let c = mesh.vertices.get(*triangle.get(2)? as usize)?;
    Some([
        model_from_object.transform_point3(DVec3::from_array(a.position.map(f64::from))),
        model_from_object.transform_point3(DVec3::from_array(b.position.map(f64::from))),
        model_from_object.transform_point3(DVec3::from_array(c.position.map(f64::from))),
    ])
}

fn triangle_may_project_to_region(
    clip_from_world: DMat4,
    viewport: ViewportSize,
    region: PickRegion,
    points: [DVec3; 3],
) -> bool {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut has_projected_point = false;

    for point in points {
        let Some((x, y, _depth)) = project_world_point(clip_from_world, viewport, point) else {
            continue;
        };
        has_projected_point = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    if !has_projected_point {
        return false;
    }

    let region_min_x = f64::from(region.x);
    let region_min_y = f64::from(region.y);
    let region_max_x = f64::from(region.x + region.width);
    let region_max_y = f64::from(region.y + region.height);

    max_x >= region_min_x && min_x <= region_max_x && max_y >= region_min_y && min_y <= region_max_y
}

fn project_world_point(
    clip_from_world: DMat4,
    viewport: ViewportSize,
    point: DVec3,
) -> Option<(f64, f64, f64)> {
    let viewport = viewport.clamped();
    let clip = clip_from_world * DVec4::new(point.x, point.y, point.z, 1.0);
    if clip.w <= f64::EPSILON {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if ndc.z < 0.0 || ndc.z > 1.0 {
        return None;
    }
    Some((
        ((ndc.x + 1.0) * 0.5) * f64::from(viewport.width),
        (1.0 - ((ndc.y + 1.0) * 0.5)) * f64::from(viewport.height),
        ndc.z,
    ))
}

fn intersect_ray_triangle(
    origin: DVec3,
    direction: DVec3,
    a: DVec3,
    b: DVec3,
    c: DVec3,
) -> Option<f64> {
    const EPSILON: f64 = 1.0e-9;
    let edge_ab = b - a;
    let edge_ac = c - a;
    let h = direction.cross(edge_ac);
    let determinant = edge_ab.dot(h);
    if determinant.abs() < EPSILON {
        return None;
    }
    let inverse_determinant = 1.0 / determinant;
    let s = origin - a;
    let u = inverse_determinant * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(edge_ab);
    let v = inverse_determinant * direction.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = inverse_determinant * edge_ac.dot(q);
    (distance > EPSILON).then_some(distance)
}

fn fit_camera_to_min_max(min: DVec3, max: DVec3) -> Camera {
    let center = (min + max) * 0.5;
    let extents = max - min;
    let radius = extents.length().max(1.0) * 0.5;
    let fov_y = 45.0_f64.to_radians();
    let distance = radius / (fov_y * 0.5).tan() + radius * 1.25;
    let view_direction =
        ((WORLD_RIGHT * 0.35) - (WORLD_FORWARD * 0.95) + (WORLD_UP * 0.7)).normalize();

    Camera {
        eye: center + (view_direction * distance),
        target: center,
        up: WORLD_UP,
        vertical_fov_degrees: 45.0,
        near_plane: 0.1,
        far_plane: (distance + radius * 8.0).max(100.0),
    }
}

pub trait RenderBackend {
    fn upload(&mut self, mesh: &PreparedMesh) -> UploadedMesh;
    fn uploads(&self) -> &[UploadedMesh];
}

#[derive(Debug)]
pub struct NullRenderBackend {
    next_mesh_id: u64,
    uploads: Vec<UploadedMesh>,
}

impl Default for NullRenderBackend {
    fn default() -> Self {
        Self {
            next_mesh_id: 1,
            uploads: Vec::new(),
        }
    }
}

impl RenderBackend for NullRenderBackend {
    fn upload(&mut self, mesh: &PreparedMesh) -> UploadedMesh {
        let gpu_vertices = mesh
            .vertices
            .iter()
            .map(|vertex| GpuVertex {
                position: vertex.position,
                normal: vertex.normal,
            })
            .collect::<Vec<_>>();

        let upload = UploadedMesh {
            mesh_id: self.next_mesh_id,
            vertex_count: gpu_vertices.len(),
            index_count: mesh.indices.len(),
            vertex_stride: GpuVertex::layout().array_stride,
            shader_entry: "vs_main",
        };
        self.next_mesh_id += 1;
        self.uploads.push(upload.clone());
        upload
    }

    fn uploads(&self) -> &[UploadedMesh] {
        &self.uploads
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct CameraUniform {
    clip_from_world: [[f32; 4]; 4],
    viewport_and_profile: [f32; 4],
    view_from_world: [[f32; 4]; 4],
}

impl CameraUniform {
    fn from_camera(camera: Camera, viewport: ViewportSize) -> Self {
        let viewport = viewport.clamped();
        Self {
            clip_from_world: camera.clip_from_world_f32(viewport).to_cols_array_2d(),
            viewport_and_profile: [
                viewport.width as f32,
                viewport.height as f32,
                ARCHITECTURAL_EDGE_WIDTH_PX * 0.5,
                SCREEN_SPACE_OUTLINE_DEPTH_THRESHOLD,
            ],
            view_from_world: mat4_from_dmat4(camera.view_from_world()).to_cols_array_2d(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct LightingUniform {
    light_direction: [f32; 4],
    factors: [f32; 4],
}

impl LightingUniform {
    fn from_defaults(defaults: RenderDefaults) -> Self {
        let direction = resolved_light_direction(defaults.directional_light.direction);

        Self {
            light_direction: [direction.x, direction.y, direction.z, 0.0],
            factors: [
                defaults.directional_light.ambient,
                defaults.directional_light.diffuse_intensity,
                0.0,
                0.0,
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct DrawUniform {
    model_from_object: [[f32; 4]; 4],
    material_color: [f32; 4],
}

impl DrawUniform {
    fn from_instance(model_from_object: DMat4, material: PreparedMaterial) -> Self {
        let color = material.color.as_rgb();

        Self {
            model_from_object: dmat4_to_f32_array(model_from_object),
            material_color: [color[0], color[1], color[2], 1.0],
        }
    }
}

#[derive(Debug)]
struct GpuMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    edge_vertex_buffer: Option<wgpu::Buffer>,
    edge_vertex_count: u32,
}

#[derive(Debug)]
struct GpuInstanceBatch {
    mesh_index: usize,
    instance_buffer: wgpu::Buffer,
    instance_count: u32,
    render_layer: RenderLayer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderLayer {
    Opaque,
    SurfaceDecal,
}

impl RenderLayer {
    fn for_render_class(class: DefaultRenderClass) -> Self {
        match class {
            DefaultRenderClass::SurfaceDecal => Self::SurfaceDecal,
            _ => Self::Opaque,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct GpuInstance {
    model_from_object: [[f32; 4]; 4],
    material_color: [f32; 4],
    pick_index: u32,
    boundary_edge_visibility: f32,
    crease_edge_visibility: f32,
    outline_index: u32,
}

impl GpuInstance {
    fn from_instance(
        model_from_object: DMat4,
        material: PreparedMaterial,
        pick_index: u32,
        default_render_class: DefaultRenderClass,
    ) -> Self {
        // Current instance transforms are assumed rigid-body or uniform-scale, so normals can
        // follow the model matrix with w=0 and be normalized once in the fragment shader.
        let draw = DrawUniform::from_instance(model_from_object, material);
        let edge_policy = edge_visibility_for_render_class(default_render_class);
        let outline_index = if object_outline_visible_for_render_class(default_render_class) {
            pick_index
        } else {
            0
        };

        Self {
            model_from_object: draw.model_from_object,
            material_color: draw.material_color,
            pick_index,
            boundary_edge_visibility: edge_policy.boundary,
            crease_edge_visibility: edge_policy.crease,
            outline_index,
        }
    }

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 9] = vertex_attr_array![
            2 => Float32x4,
            3 => Float32x4,
            4 => Float32x4,
            5 => Float32x4,
            6 => Float32x4,
            7 => Uint32,
            10 => Float32,
            11 => Float32,
            12 => Uint32
        ];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRIBUTES,
        }
    }
}

#[derive(Debug)]
pub struct DepthTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl DepthTarget {
    pub fn with_defaults(
        device: &wgpu::Device,
        viewport: ViewportSize,
        defaults: RenderDefaults,
        label: &'static str,
    ) -> Self {
        let viewport = viewport.clamped();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: viewport.width,
                height: viewport.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: defaults.depth_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(label),
            ..Default::default()
        });

        Self {
            _texture: texture,
            view,
        }
    }

    pub fn with_label(device: &wgpu::Device, viewport: ViewportSize, label: &'static str) -> Self {
        Self::with_defaults(device, viewport, RenderDefaults::default(), label)
    }

    pub fn new(device: &wgpu::Device, viewport: ViewportSize) -> Self {
        Self::with_label(device, viewport, "w depth target")
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

#[derive(Debug)]
struct ScreenSpaceOutlineTargets {
    _object_id_texture: wgpu::Texture,
    object_id_view: wgpu::TextureView,
}

impl ScreenSpaceOutlineTargets {
    fn new(device: &wgpu::Device, viewport: ViewportSize) -> Self {
        let viewport = viewport.clamped();
        let object_id_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w screen-space outline object-id texture"),
            size: wgpu::Extent3d {
                width: viewport.width,
                height: viewport.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCREEN_SPACE_OBJECT_ID_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let object_id_view = object_id_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("w screen-space outline object-id texture view"),
            ..Default::default()
        });

        Self {
            _object_id_texture: object_id_texture,
            object_id_view,
        }
    }
}

#[derive(Debug)]
struct ScreenSpaceAmbientOcclusionTargets {
    _normal_texture: wgpu::Texture,
    normal_view: wgpu::TextureView,
}

impl ScreenSpaceAmbientOcclusionTargets {
    fn new(device: &wgpu::Device, viewport: ViewportSize) -> Self {
        let viewport = viewport.clamped();
        let normal_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w screen-space ao normal texture"),
            size: wgpu::Extent3d {
                width: viewport.width,
                height: viewport.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCREEN_SPACE_NORMAL_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let normal_view = normal_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("w screen-space ao normal texture view"),
            ..Default::default()
        });

        Self {
            _normal_texture: normal_texture,
            normal_view,
        }
    }
}

#[derive(Debug)]
pub struct MeshRenderer {
    pipeline: wgpu::RenderPipeline,
    architectural_pipeline: wgpu::RenderPipeline,
    surface_decal_pipeline: wgpu::RenderPipeline,
    architectural_surface_decal_pipeline: wgpu::RenderPipeline,
    reference_grid_pipeline: wgpu::RenderPipeline,
    normal_pipeline: wgpu::RenderPipeline,
    ssao_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    crease_edge_pipeline: wgpu::RenderPipeline,
    outline_id_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    pick_pipeline: wgpu::RenderPipeline,
    pick_surface_decal_pipeline: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    _lighting_buffer: wgpu::Buffer,
    scene_bind_group: wgpu::BindGroup,
    outline_bind_group_layout: wgpu::BindGroupLayout,
    ssao_bind_group_layout: wgpu::BindGroupLayout,
    outline_targets: ScreenSpaceOutlineTargets,
    ssao_targets: ScreenSpaceAmbientOcclusionTargets,
    viewport: ViewportSize,
    camera: Camera,
    defaults: RenderDefaults,
    profile: RenderProfileId,
    reference_grid_visible: bool,
    reference_grid_vertex_buffer: Option<wgpu::Buffer>,
    reference_grid_vertex_count: u32,
    meshes: Vec<GpuMesh>,
    instance_batches: Vec<GpuInstanceBatch>,
    pick_targets: Vec<PickHit>,
    next_mesh_id: u64,
}

impl MeshRenderer {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        viewport: ViewportSize,
        camera: Camera,
    ) -> Self {
        Self::with_defaults(
            device,
            color_format,
            viewport,
            camera,
            RenderDefaults::default(),
        )
    }

    pub fn with_defaults(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        viewport: ViewportSize,
        camera: Camera,
        defaults: RenderDefaults,
    ) -> Self {
        let camera_uniform = CameraUniform::from_camera(camera, viewport);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("w camera buffer"),
            contents: bytes_of(&camera_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let lighting_uniform = LightingUniform::from_defaults(defaults);
        let lighting_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("w lighting buffer"),
            contents: bytes_of(&lighting_uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let scene_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("w scene bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("w scene bind group"),
            layout: &scene_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: lighting_buffer.as_entire_binding(),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("w mesh pipeline layout"),
            bind_group_layouts: &[Some(&scene_bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w basic mesh shader"),
            source: wgpu::ShaderSource::Wgsl(BASIC_MESH_SHADER_WGSL.into()),
        });
        let architectural_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w architectural mesh shader"),
            source: wgpu::ShaderSource::Wgsl(ARCHITECTURAL_MESH_SHADER_WGSL.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuVertex::layout(), GpuInstance::layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: defaults.front_face,
                cull_mode: defaults.cull_mode,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: defaults.depth_format,
                depth_write_enabled: Some(defaults.depth_write_enabled),
                depth_compare: Some(defaults.depth_compare),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let architectural_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("w architectural mesh pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &architectural_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[GpuVertex::layout(), GpuInstance::layout()],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: defaults.front_face,
                    cull_mode: defaults.cull_mode,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: defaults.depth_format,
                    depth_write_enabled: Some(defaults.depth_write_enabled),
                    depth_compare: Some(defaults.depth_compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &architectural_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let surface_decal_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("w surface decal mesh pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[GpuVertex::layout(), GpuInstance::layout()],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: defaults.front_face,
                    cull_mode: defaults.cull_mode,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: defaults.depth_format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(depth_compare_equal_variant(defaults.depth_compare)),
                    stencil: wgpu::StencilState::default(),
                    bias: surface_decal_depth_bias(defaults.depth_compare),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let architectural_surface_decal_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("w architectural surface decal mesh pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &architectural_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[GpuVertex::layout(), GpuInstance::layout()],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: defaults.front_face,
                    cull_mode: defaults.cull_mode,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: defaults.depth_format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(depth_compare_equal_variant(defaults.depth_compare)),
                    stencil: wgpu::StencilState::default(),
                    bias: surface_decal_depth_bias(defaults.depth_compare),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &architectural_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let reference_grid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w reference grid shader"),
            source: wgpu::ShaderSource::Wgsl(REFERENCE_GRID_SHADER_WGSL.into()),
        });
        let reference_grid_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("w reference grid pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &reference_grid_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[GpuReferenceGridVertex::layout()],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::LineList,
                    strip_index_format: None,
                    front_face: defaults.front_face,
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: defaults.depth_format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(defaults.depth_compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &reference_grid_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let normal_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w screen-space ao normal shader"),
            source: wgpu::ShaderSource::Wgsl(NORMAL_MESH_SHADER_WGSL.into()),
        });
        let normal_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w screen-space ao normal pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &normal_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuVertex::layout(), GpuInstance::layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: defaults.front_face,
                cull_mode: defaults.cull_mode,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: defaults.depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(depth_compare_equal_variant(defaults.depth_compare)),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &normal_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: SCREEN_SPACE_NORMAL_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let ssao_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("w screen-space ao bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        let ssao_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("w screen-space ao pipeline layout"),
            bind_group_layouts: &[Some(&ssao_bind_group_layout)],
            immediate_size: 0,
        });
        let ssao_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w screen-space ao shader"),
            source: wgpu::ShaderSource::Wgsl(SCREEN_SPACE_AO_SHADER_WGSL.into()),
        });
        let ssao_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w screen-space ao pipeline"),
            layout: Some(&ssao_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &ssao_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &ssao_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let edge_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w edge ribbon shader"),
            source: wgpu::ShaderSource::Wgsl(EDGE_RIBBON_SHADER_WGSL.into()),
        });
        let edge_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w edge ribbon pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &edge_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuEdgeVertex::layout(), GpuInstance::layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: defaults.front_face,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: defaults.depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(depth_compare_equal_variant(defaults.depth_compare)),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &edge_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let crease_edge_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w crease ribbon shader"),
            source: wgpu::ShaderSource::Wgsl(CREASE_RIBBON_SHADER_WGSL.into()),
        });
        let crease_edge_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w crease ribbon pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &crease_edge_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuEdgeVertex::layout(), GpuInstance::layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: defaults.front_face,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: defaults.depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(depth_compare_equal_variant(defaults.depth_compare)),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &crease_edge_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let outline_id_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w screen-space outline object-id shader"),
            source: wgpu::ShaderSource::Wgsl(OBJECT_ID_MESH_SHADER_WGSL.into()),
        });
        let outline_id_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w screen-space outline object-id pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &outline_id_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuVertex::layout(), GpuInstance::layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: defaults.front_face,
                cull_mode: defaults.cull_mode,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: defaults.depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(depth_compare_equal_variant(defaults.depth_compare)),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &outline_id_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: SCREEN_SPACE_OBJECT_ID_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let outline_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("w screen-space outline bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Uint,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        let outline_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("w screen-space outline pipeline layout"),
                bind_group_layouts: &[Some(&outline_bind_group_layout)],
                immediate_size: 0,
            });
        let outline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w screen-space outline shader"),
            source: wgpu::ShaderSource::Wgsl(SCREEN_SPACE_OUTLINE_SHADER_WGSL.into()),
        });
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w screen-space outline pipeline"),
            layout: Some(&outline_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &outline_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &outline_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let pick_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w pick mesh shader"),
            source: wgpu::ShaderSource::Wgsl(PICK_MESH_SHADER_WGSL.into()),
        });
        let pick_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w pick mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &pick_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuVertex::layout(), GpuInstance::layout()],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: defaults.front_face,
                cull_mode: defaults.cull_mode,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: defaults.depth_format,
                depth_write_enabled: Some(defaults.depth_write_enabled),
                depth_compare: Some(defaults.depth_compare),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &pick_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Uint,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let pick_surface_decal_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("w pick surface decal mesh pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &pick_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[GpuVertex::layout(), GpuInstance::layout()],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: defaults.front_face,
                    cull_mode: defaults.cull_mode,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: defaults.depth_format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(depth_compare_equal_variant(defaults.depth_compare)),
                    stencil: wgpu::StencilState::default(),
                    bias: surface_decal_depth_bias(defaults.depth_compare),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &pick_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Uint,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });

        Self {
            pipeline,
            architectural_pipeline,
            surface_decal_pipeline,
            architectural_surface_decal_pipeline,
            reference_grid_pipeline,
            normal_pipeline,
            ssao_pipeline,
            edge_pipeline,
            crease_edge_pipeline,
            outline_id_pipeline,
            outline_pipeline,
            pick_pipeline,
            pick_surface_decal_pipeline,
            camera_buffer,
            _lighting_buffer: lighting_buffer,
            scene_bind_group,
            outline_bind_group_layout,
            ssao_bind_group_layout,
            outline_targets: ScreenSpaceOutlineTargets::new(device, viewport),
            ssao_targets: ScreenSpaceAmbientOcclusionTargets::new(device, viewport),
            viewport: viewport.clamped(),
            camera,
            defaults,
            profile: RenderProfileId::Diffuse,
            reference_grid_visible: false,
            reference_grid_vertex_buffer: None,
            reference_grid_vertex_count: 0,
            meshes: Vec::new(),
            instance_batches: Vec::new(),
            pick_targets: Vec::new(),
            next_mesh_id: 1,
        }
    }

    pub fn upload_prepared_mesh(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &PreparedMesh,
    ) -> UploadedMesh {
        self.upload_prepared_scene(
            device,
            queue,
            &PreparedRenderScene {
                bounds: mesh.bounds,
                definitions: vec![PreparedRenderDefinition {
                    id: GeometryDefinitionId(1),
                    mesh: mesh.clone(),
                }],
                instances: vec![PreparedRenderInstance {
                    id: GeometryInstanceId(1),
                    element_id: SemanticElementId::new("mesh/instance"),
                    definition_id: GeometryDefinitionId(1),
                    model_from_object: DMat4::IDENTITY,
                    world_bounds: mesh.bounds,
                    material: PreparedMaterial::default(),
                    default_render_class: DefaultRenderClass::Physical,
                }],
            },
        )
        .into_iter()
        .next()
        .expect("single-mesh upload should produce one upload")
    }

    pub fn upload_prepared_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &PreparedRenderScene,
    ) -> Vec<UploadedMesh> {
        self.meshes.clear();
        self.instance_batches.clear();
        self.pick_targets.clear();
        self.reference_grid_vertex_buffer = None;
        self.reference_grid_vertex_count = 0;

        let uploads = scene
            .definitions
            .iter()
            .map(|definition| self.upload_mesh_definition(device, definition))
            .collect::<Vec<_>>();

        let mut opaque_instances_by_mesh = vec![Vec::new(); scene.definitions.len()];
        let mut surface_decal_instances_by_mesh = vec![Vec::new(); scene.definitions.len()];
        for instance in &scene.instances {
            let mesh_index = scene
                .definitions
                .iter()
                .position(|definition| definition.id == instance.definition_id)
                .expect("render scene instance references an uploaded definition");
            let local_origin = scene.definitions[mesh_index].mesh.local_origin;
            let model_from_object =
                instance.model_from_object * DMat4::from_translation(local_origin);
            let pick_index = self.pick_targets.len() as u32 + 1;
            let world_centroid = instance.world_bounds.center();
            self.pick_targets.push(PickHit {
                instance_id: instance.id,
                element_id: instance.element_id.clone(),
                definition_id: instance.definition_id,
                world_centroid,
                world_anchor: world_centroid,
            });
            let gpu_instance = GpuInstance::from_instance(
                model_from_object,
                instance.material,
                pick_index,
                instance.default_render_class,
            );
            match RenderLayer::for_render_class(instance.default_render_class) {
                RenderLayer::Opaque => opaque_instances_by_mesh[mesh_index].push(gpu_instance),
                RenderLayer::SurfaceDecal => {
                    surface_decal_instances_by_mesh[mesh_index].push(gpu_instance);
                }
            }
        }

        for (mesh_index, instances) in opaque_instances_by_mesh.into_iter().enumerate() {
            if instances.is_empty() {
                continue;
            }
            self.upload_instance_batch(device, mesh_index, RenderLayer::Opaque, &instances);
        }
        for (mesh_index, instances) in surface_decal_instances_by_mesh.into_iter().enumerate() {
            if instances.is_empty() {
                continue;
            }
            self.upload_instance_batch(device, mesh_index, RenderLayer::SurfaceDecal, &instances);
        }

        self.upload_reference_grid(device, scene.bounds);
        self.update_camera(queue);
        uploads
    }

    fn upload_reference_grid(&mut self, device: &wgpu::Device, bounds: Bounds3) {
        let vertices = reference_grid_vertices(bounds);
        self.reference_grid_vertex_count = vertices.len() as u32;
        self.reference_grid_vertex_buffer = (!vertices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("w reference grid vertex buffer"),
                contents: cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
    }

    fn upload_mesh_definition(
        &mut self,
        device: &wgpu::Device,
        definition: &PreparedRenderDefinition,
    ) -> UploadedMesh {
        let mesh = &definition.mesh;
        let gpu_vertices = mesh
            .vertices
            .iter()
            .map(|vertex| GpuVertex {
                position: vertex.position,
                normal: vertex.normal,
            })
            .collect::<Vec<_>>();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("w mesh vertex buffer"),
            contents: cast_slice(&gpu_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("w mesh index buffer"),
            contents: cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let edges = ExtractedMeshEdges::extract(mesh, architectural_edge_extraction_config());
        let edge_vertices = edge_ribbon_vertices(mesh, &edges);
        let edge_vertex_count = edge_vertices.len() as u32;
        let edge_vertex_buffer = (!edge_vertices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("w mesh edge ribbon vertex buffer"),
                contents: cast_slice(&edge_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });

        let upload = UploadedMesh {
            mesh_id: self.next_mesh_id,
            vertex_count: gpu_vertices.len(),
            index_count: mesh.indices.len(),
            vertex_stride: GpuVertex::layout().array_stride,
            shader_entry: "vs_main",
        };
        self.next_mesh_id += 1;
        self.meshes.push(GpuMesh {
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            edge_vertex_buffer,
            edge_vertex_count,
        });

        upload
    }

    fn upload_instance_batch(
        &mut self,
        device: &wgpu::Device,
        mesh_index: usize,
        render_layer: RenderLayer,
        instances: &[GpuInstance],
    ) {
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("w instance buffer"),
            contents: cast_slice(instances),
            usage: wgpu::BufferUsages::VERTEX,
        });

        self.instance_batches.push(GpuInstanceBatch {
            mesh_index,
            instance_buffer,
            instance_count: instances.len() as u32,
            render_layer,
        });
    }

    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, viewport: ViewportSize) {
        self.viewport = viewport.clamped();
        self.outline_targets = ScreenSpaceOutlineTargets::new(device, self.viewport);
        self.ssao_targets = ScreenSpaceAmbientOcclusionTargets::new(device, self.viewport);
        self.update_camera(queue);
    }

    pub fn set_camera(&mut self, queue: &wgpu::Queue, camera: Camera) {
        self.camera = camera;
        self.update_camera(queue);
    }

    pub fn camera(&self) -> Camera {
        self.camera
    }

    pub fn defaults(&self) -> RenderDefaults {
        self.defaults
    }

    pub fn available_profiles(&self) -> &'static [RenderProfileDescriptor] {
        available_render_profiles()
    }

    pub fn profile(&self) -> RenderProfileId {
        self.profile
    }

    pub fn set_profile(&mut self, profile: RenderProfileId) {
        self.profile = profile;
    }

    pub fn reference_grid_visible(&self) -> bool {
        self.reference_grid_visible
    }

    pub fn set_reference_grid_visible(&mut self, visible: bool) {
        self.reference_grid_visible = visible;
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
    ) {
        self.render_with_clear_color(encoder, target, depth_target, self.defaults.clear_color);
    }

    pub fn render_with_device(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
    ) {
        self.render_with_clear_color_and_device(
            device,
            encoder,
            target,
            depth_target,
            self.defaults.clear_color,
        );
    }

    pub fn render_with_clear_color(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
        clear_color: wgpu::Color,
    ) {
        self.render_with_optional_device(None, encoder, target, depth_target, clear_color);
    }

    pub fn render_with_clear_color_and_device(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
        clear_color: wgpu::Color,
    ) {
        self.render_with_optional_device(Some(device), encoder, target, depth_target, clear_color);
    }

    fn render_with_optional_device(
        &self,
        device: Option<&wgpu::Device>,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
        clear_color: wgpu::Color,
    ) {
        if self.instance_batches.is_empty() {
            return;
        }

        let needs_depth_sampling = matches!(
            self.profile,
            RenderProfileId::ArchitecturalV2
                | RenderProfileId::ArchitecturalV3
                | RenderProfileId::ArchitecturalV4
        ) && device.is_some();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("w mesh pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_target,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.defaults.depth_clear_value),
                        store: if needs_depth_sampling {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mesh_pipeline = if uses_architectural_surface_lighting(self.profile) {
                &self.architectural_pipeline
            } else {
                &self.pipeline
            };
            let surface_decal_pipeline = if uses_architectural_surface_lighting(self.profile) {
                &self.architectural_surface_decal_pipeline
            } else {
                &self.surface_decal_pipeline
            };
            pass.set_pipeline(mesh_pipeline);
            pass.set_bind_group(0, &self.scene_bind_group, &[]);

            for batch in self
                .instance_batches
                .iter()
                .filter(|batch| batch.render_layer == RenderLayer::Opaque)
            {
                let mesh = &self.meshes[batch.mesh_index];
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, batch.instance_buffer.slice(..));
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..batch.instance_count);
            }

            pass.set_pipeline(surface_decal_pipeline);
            for batch in self
                .instance_batches
                .iter()
                .filter(|batch| batch.render_layer == RenderLayer::SurfaceDecal)
            {
                let mesh = &self.meshes[batch.mesh_index];
                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, batch.instance_buffer.slice(..));
                pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..batch.instance_count);
            }

            if self.reference_grid_visible {
                self.draw_reference_grid(&mut pass);
            }

            if self.profile == RenderProfileId::ArchitecturalV1 {
                self.draw_mesh_edges(&mut pass);
            }
            if self.profile == RenderProfileId::ArchitecturalV3 {
                self.draw_mesh_crease_edges(&mut pass);
            }
        }

        if self.profile == RenderProfileId::ArchitecturalV4 {
            if let Some(device) = device {
                self.render_normal_buffer(encoder, depth_target);
                self.render_screen_space_ambient_occlusion(device, encoder, target, depth_target);
                self.render_mesh_crease_edges_overlay(encoder, target, depth_target);
            }
        }

        if uses_screen_space_outline(self.profile) {
            if let Some(device) = device {
                self.render_outline_object_ids(encoder, depth_target);
                self.render_screen_space_outline(device, encoder, target, depth_target);
            }
        }
    }

    fn draw_reference_grid<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        let Some(vertex_buffer) = &self.reference_grid_vertex_buffer else {
            return;
        };
        if self.reference_grid_vertex_count == 0 {
            return;
        }

        pass.set_pipeline(&self.reference_grid_pipeline);
        pass.set_bind_group(0, &self.scene_bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..self.reference_grid_vertex_count, 0..1);
    }

    fn draw_mesh_edges<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        self.draw_mesh_edges_with_pipeline(pass, &self.edge_pipeline);
    }

    fn draw_mesh_crease_edges<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        self.draw_mesh_edges_with_pipeline(pass, &self.crease_edge_pipeline);
    }

    fn draw_mesh_edges_with_pipeline<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        pipeline: &'pass wgpu::RenderPipeline,
    ) {
        pass.set_pipeline(pipeline);
        for batch in self
            .instance_batches
            .iter()
            .filter(|batch| batch.render_layer == RenderLayer::Opaque)
        {
            let mesh = &self.meshes[batch.mesh_index];
            let Some(edge_vertex_buffer) = &mesh.edge_vertex_buffer else {
                continue;
            };
            pass.set_vertex_buffer(0, edge_vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, batch.instance_buffer.slice(..));
            pass.draw(0..mesh.edge_vertex_count, 0..batch.instance_count);
        }
    }

    fn render_mesh_crease_edges_overlay(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("w crease edge overlay pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_target,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.scene_bind_group, &[]);
        self.draw_mesh_crease_edges(&mut pass);
    }

    fn render_normal_buffer(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        depth_target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("w screen-space ao normal pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.ssao_targets.normal_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.5,
                        g: 0.5,
                        b: 1.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_target,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.normal_pipeline);
        pass.set_bind_group(0, &self.scene_bind_group, &[]);

        for batch in self
            .instance_batches
            .iter()
            .filter(|batch| batch.render_layer == RenderLayer::Opaque)
        {
            let mesh = &self.meshes[batch.mesh_index];
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, batch.instance_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..batch.instance_count);
        }
    }

    fn render_screen_space_ambient_occlusion(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("w screen-space ao bind group"),
            layout: &self.ssao_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_target),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.ssao_targets.normal_view),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("w screen-space ao pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.ssao_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn render_outline_object_ids(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        depth_target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("w screen-space outline object-id pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.outline_targets.object_id_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_target,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.outline_id_pipeline);
        pass.set_bind_group(0, &self.scene_bind_group, &[]);

        for batch in self
            .instance_batches
            .iter()
            .filter(|batch| batch.render_layer == RenderLayer::Opaque)
        {
            let mesh = &self.meshes[batch.mesh_index];
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, batch.instance_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..batch.instance_count);
        }
    }

    fn render_screen_space_outline(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("w screen-space outline bind group"),
            layout: &self.outline_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_target),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &self.outline_targets.object_id_view,
                    ),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("w screen-space outline pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.outline_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn render_pick_region(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
        region: PickRegion,
    ) -> Option<PickRegion> {
        let region = clamp_pick_region(region, self.viewport)?;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("w pick mesh pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_target,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(self.defaults.depth_clear_value),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_scissor_rect(region.x, region.y, region.width, region.height);
        pass.set_pipeline(&self.pick_pipeline);
        pass.set_bind_group(0, &self.scene_bind_group, &[]);

        for batch in self
            .instance_batches
            .iter()
            .filter(|batch| batch.render_layer == RenderLayer::Opaque)
        {
            let mesh = &self.meshes[batch.mesh_index];
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, batch.instance_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..batch.instance_count);
        }

        pass.set_pipeline(&self.pick_surface_decal_pipeline);
        for batch in self
            .instance_batches
            .iter()
            .filter(|batch| batch.render_layer == RenderLayer::SurfaceDecal)
        {
            let mesh = &self.meshes[batch.mesh_index];
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, batch.instance_buffer.slice(..));
            pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..batch.instance_count);
        }

        Some(region)
    }

    pub fn decode_pick_pixels(&self, region: PickRegion, rgba8: &[u8]) -> PickResult {
        Self::decode_pick_pixels_with_targets(region, rgba8, &self.pick_targets)
    }

    pub fn decode_pick_pixels_with_targets(
        region: PickRegion,
        rgba8: &[u8],
        pick_targets: &[PickHit],
    ) -> PickResult {
        let mut seen = HashSet::new();
        let mut hits = Vec::new();

        for pixel in rgba8.chunks_exact(4) {
            let pick_index = decode_pick_index(pixel);
            if pick_index == 0 || !seen.insert(pick_index) {
                continue;
            }
            let Some(hit) = pick_targets.get((pick_index - 1) as usize) else {
                continue;
            };
            hits.push(hit.clone());
        }

        PickResult { region, hits }
    }

    pub fn pick_targets(&self) -> &[PickHit] {
        &self.pick_targets
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn pick_region(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        region: PickRegion,
    ) -> Result<PickResult, PickError> {
        let Some(region) = clamp_pick_region(region, self.viewport) else {
            return Ok(PickResult::empty(region));
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w pick texture"),
            size: wgpu::Extent3d {
                width: self.viewport.width,
                height: self.viewport.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Uint,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("w pick texture view"),
            ..Default::default()
        });
        let depth_target =
            DepthTarget::with_defaults(device, self.viewport, self.defaults, "w pick depth target");

        let unpadded_bytes_per_row = region
            .width
            .checked_mul(4)
            .ok_or(PickError::OutputTooLarge)?;
        let padded_bytes_per_row =
            align_to(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback_size = u64::from(padded_bytes_per_row)
            .checked_mul(u64::from(region.height))
            .ok_or(PickError::OutputTooLarge)?;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("w pick readback buffer"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("w pick encoder"),
        });
        self.render_pick_region(&mut encoder, &view, depth_target.view(), region);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: region.x,
                    y: region.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(region.height),
                },
            },
            wgpu::Extent3d {
                width: region.width,
                height: region.height,
                depth_or_array_layers: 1,
            },
        );

        let submission = queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        let mapping_result = rx.recv().map_err(|_| PickError::ReadbackChannelClosed)?;
        mapping_result?;

        let mapped = slice.get_mapped_range();
        let rgba8 = strip_padded_rows(
            &mapped,
            unpadded_bytes_per_row as usize,
            padded_bytes_per_row as usize,
            region.height as usize,
        );
        drop(mapped);
        readback.unmap();

        Ok(self.decode_pick_pixels(region, &rgba8))
    }

    fn update_camera(&self, queue: &wgpu::Queue) {
        let uniform = CameraUniform::from_camera(self.camera, self.viewport);
        queue.write_buffer(&self.camera_buffer, 0, bytes_of(&uniform));
    }
}

fn clamp_pick_region(region: PickRegion, viewport: ViewportSize) -> Option<PickRegion> {
    let viewport = viewport.clamped();
    if region.is_empty() || region.x >= viewport.width || region.y >= viewport.height {
        return None;
    }

    Some(PickRegion {
        x: region.x,
        y: region.y,
        width: region.width.min(viewport.width - region.x),
        height: region.height.min(viewport.height - region.y),
    })
}

fn decode_pick_index(pixel: &[u8]) -> u32 {
    u32::from(pixel[0])
        | (u32::from(pixel[1]) << 8)
        | (u32::from(pixel[2]) << 16)
        | (u32::from(pixel[3]) << 24)
}

fn architectural_edge_extraction_config() -> MeshEdgeExtractionConfig {
    MeshEdgeExtractionConfig {
        crease_angle_radians: ARCHITECTURAL_CREASE_ANGLE_DEGREES.to_radians(),
    }
}

fn depth_compare_equal_variant(compare: wgpu::CompareFunction) -> wgpu::CompareFunction {
    match compare {
        wgpu::CompareFunction::Greater | wgpu::CompareFunction::GreaterEqual => {
            wgpu::CompareFunction::GreaterEqual
        }
        _ => wgpu::CompareFunction::LessEqual,
    }
}

fn surface_decal_depth_bias(compare: wgpu::CompareFunction) -> wgpu::DepthBiasState {
    let direction = match compare {
        wgpu::CompareFunction::Greater | wgpu::CompareFunction::GreaterEqual => 1,
        _ => -1,
    };

    wgpu::DepthBiasState {
        constant: direction,
        slope_scale: direction as f32,
        clamp: 0.0,
    }
}

fn reference_grid_vertices(bounds: Bounds3) -> Vec<GpuReferenceGridVertex> {
    let size = bounds.size();
    let span = size.x.abs().max(size.y.abs()).max(0.01);
    let spacing = metric_reference_grid_spacing(span / 16.0);
    let major_spacing = spacing * 10.0;
    let center = bounds.center();
    let half_extent = (span * 0.85).max(major_spacing * 2.0);
    let min_x = ((center.x - half_extent) / spacing).floor() * spacing;
    let max_x = ((center.x + half_extent) / spacing).ceil() * spacing;
    let min_y = ((center.y - half_extent) / spacing).floor() * spacing;
    let max_y = ((center.y + half_extent) / spacing).ceil() * spacing;
    let z = bounds.min.z - (span * 0.006).max(0.025);
    let mut vertices = Vec::new();

    let mut x = min_x;
    while x <= max_x + spacing * 0.5 {
        let alpha = reference_grid_line_alpha(x, major_spacing);
        vertices.push(GpuReferenceGridVertex {
            position: [x as f32, min_y as f32, z as f32],
            alpha,
        });
        vertices.push(GpuReferenceGridVertex {
            position: [x as f32, max_y as f32, z as f32],
            alpha,
        });
        x += spacing;
    }

    let mut y = min_y;
    while y <= max_y + spacing * 0.5 {
        let alpha = reference_grid_line_alpha(y, major_spacing);
        vertices.push(GpuReferenceGridVertex {
            position: [min_x as f32, y as f32, z as f32],
            alpha,
        });
        vertices.push(GpuReferenceGridVertex {
            position: [max_x as f32, y as f32, z as f32],
            alpha,
        });
        y += spacing;
    }

    vertices
}

fn reference_grid_line_alpha(coordinate: f64, major_spacing: f64) -> f32 {
    let major = (coordinate / major_spacing).round() * major_spacing;
    if (coordinate - major).abs() <= major_spacing * 0.02 {
        0.58
    } else {
        0.32
    }
}

fn metric_reference_grid_spacing(target: f64) -> f64 {
    if !target.is_finite() || target <= 0.0 {
        return 1.0;
    }

    10.0_f64.powf(target.log10().round())
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RenderClassEdgeVisibility {
    boundary: f32,
    crease: f32,
}

fn edge_visibility_for_render_class(class: DefaultRenderClass) -> RenderClassEdgeVisibility {
    match class {
        DefaultRenderClass::Terrain => RenderClassEdgeVisibility {
            boundary: 0.0,
            crease: 0.0,
        },
        DefaultRenderClass::TerrainFeature => RenderClassEdgeVisibility {
            boundary: 0.0,
            crease: 1.0,
        },
        DefaultRenderClass::Water => RenderClassEdgeVisibility {
            boundary: 0.0,
            crease: 0.0,
        },
        DefaultRenderClass::SurfaceDecal => RenderClassEdgeVisibility {
            boundary: 0.0,
            crease: 0.0,
        },
        DefaultRenderClass::Vegetation => RenderClassEdgeVisibility {
            boundary: 1.0,
            crease: 0.0,
        },
        DefaultRenderClass::VegetationCover => RenderClassEdgeVisibility {
            boundary: 0.0,
            crease: 0.0,
        },
        _ => RenderClassEdgeVisibility {
            boundary: 1.0,
            crease: 1.0,
        },
    }
}

fn object_outline_visible_for_render_class(class: DefaultRenderClass) -> bool {
    !matches!(
        class,
        DefaultRenderClass::Terrain
            | DefaultRenderClass::VegetationCover
            | DefaultRenderClass::Water
            | DefaultRenderClass::SurfaceDecal
    )
}

fn uses_architectural_surface_lighting(profile: RenderProfileId) -> bool {
    matches!(
        profile,
        RenderProfileId::ArchitecturalV1
            | RenderProfileId::ArchitecturalV2
            | RenderProfileId::ArchitecturalV3
            | RenderProfileId::ArchitecturalV4
    )
}

fn uses_screen_space_outline(profile: RenderProfileId) -> bool {
    matches!(
        profile,
        RenderProfileId::ArchitecturalV2
            | RenderProfileId::ArchitecturalV3
            | RenderProfileId::ArchitecturalV4
    )
}

fn edge_ribbon_vertices(mesh: &PreparedMesh, edges: &ExtractedMeshEdges) -> Vec<GpuEdgeVertex> {
    let edge_count = edges.boundary_edges.len() + edges.crease_edges.len();
    let mut vertices = Vec::with_capacity(edge_count * 6);

    for [start_index, end_index] in edges.boundary_edges.iter().copied() {
        append_edge_ribbon_vertices(&mut vertices, mesh, start_index, end_index, 0.0);
    }
    for [start_index, end_index] in edges.crease_edges.iter().copied() {
        append_edge_ribbon_vertices(&mut vertices, mesh, start_index, end_index, 1.0);
    }

    vertices
}

fn append_edge_ribbon_vertices(
    vertices: &mut Vec<GpuEdgeVertex>,
    mesh: &PreparedMesh,
    start_index: u32,
    end_index: u32,
    edge_kind: f32,
) {
    let Some(start) = mesh.vertices.get(start_index as usize) else {
        return;
    };
    let Some(end) = mesh.vertices.get(end_index as usize) else {
        return;
    };
    let start = start.position;
    let end = end.position;

    vertices.extend_from_slice(&[
        GpuEdgeVertex::new(start, end, [0.0, -1.0], edge_kind),
        GpuEdgeVertex::new(start, end, [1.0, -1.0], edge_kind),
        GpuEdgeVertex::new(start, end, [1.0, 1.0], edge_kind),
        GpuEdgeVertex::new(start, end, [0.0, -1.0], edge_kind),
        GpuEdgeVertex::new(start, end, [1.0, 1.0], edge_kind),
        GpuEdgeVertex::new(start, end, [0.0, 1.0], edge_kind),
    ]);
}

#[cfg(test)]
fn encode_pick_index(index: u32) -> [u8; 4] {
    [
        (index & 0xff) as u8,
        ((index >> 8) & 0xff) as u8,
        ((index >> 16) & 0xff) as u8,
        ((index >> 24) & 0xff) as u8,
    ]
}

pub fn default_clear_color() -> wgpu::Color {
    RenderDefaults::default().clear_color
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedImage {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
impl RenderedImage {
    pub fn has_variation(&self) -> bool {
        self.rgba8
            .chunks_exact(4)
            .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 1
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, thiserror::Error)]
pub enum HeadlessRenderError {
    #[error(transparent)]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error(transparent)]
    Device(#[from] wgpu::RequestDeviceError),
    #[error(transparent)]
    BufferAsync(#[from] wgpu::BufferAsyncError),
    #[error(transparent)]
    Poll(#[from] wgpu::PollError),
    #[error("failed to receive the GPU readback callback")]
    ReadbackChannelClosed,
    #[error("the requested output size is too large for a readback buffer")]
    OutputTooLarge,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, thiserror::Error)]
pub enum PickError {
    #[error(transparent)]
    BufferAsync(#[from] wgpu::BufferAsyncError),
    #[error(transparent)]
    Poll(#[from] wgpu::PollError),
    #[error("failed to receive the GPU pick readback callback")]
    ReadbackChannelClosed,
    #[error("the requested pick region is too large for a readback buffer")]
    OutputTooLarge,
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn render_prepared_mesh_offscreen(
    mesh: &PreparedMesh,
    viewport: ViewportSize,
    camera: Camera,
) -> Result<RenderedImage, HeadlessRenderError> {
    render_prepared_scene_offscreen(
        &PreparedRenderScene {
            bounds: mesh.bounds,
            definitions: vec![PreparedRenderDefinition {
                id: GeometryDefinitionId(1),
                mesh: mesh.clone(),
            }],
            instances: vec![PreparedRenderInstance {
                id: GeometryInstanceId(1),
                element_id: SemanticElementId::new("mesh/instance"),
                definition_id: GeometryDefinitionId(1),
                model_from_object: DMat4::IDENTITY,
                world_bounds: mesh.bounds,
                material: PreparedMaterial::default(),
                default_render_class: DefaultRenderClass::Physical,
            }],
        },
        viewport,
        camera,
    )
    .await
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn render_prepared_scene_offscreen(
    scene: &PreparedRenderScene,
    viewport: ViewportSize,
    camera: Camera,
) -> Result<RenderedImage, HeadlessRenderError> {
    let viewport = viewport.clamped();
    let defaults = RenderDefaults::default();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await?;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("w headless device"),
            ..Default::default()
        })
        .await?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("w offscreen texture"),
        size: wgpu::Extent3d {
            width: viewport.width,
            height: viewport.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_target =
        DepthTarget::with_defaults(&device, viewport, defaults, "w offscreen depth target");
    let mut renderer = MeshRenderer::with_defaults(&device, format, viewport, camera, defaults);
    renderer.upload_prepared_scene(&device, &queue, scene);

    let unpadded_bytes_per_row = viewport
        .width
        .checked_mul(4)
        .ok_or(HeadlessRenderError::OutputTooLarge)?;
    let padded_bytes_per_row = align_to(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback_size = u64::from(padded_bytes_per_row)
        .checked_mul(u64::from(viewport.height))
        .ok_or(HeadlessRenderError::OutputTooLarge)?;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("w readback buffer"),
        size: readback_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("w offscreen encoder"),
    });
    renderer.render_with_device(&device, &mut encoder, &view, depth_target.view());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(viewport.height),
            },
        },
        wgpu::Extent3d {
            width: viewport.width,
            height: viewport.height,
            depth_or_array_layers: 1,
        },
    );

    let submission = queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    let mapping_result = rx
        .recv()
        .map_err(|_| HeadlessRenderError::ReadbackChannelClosed)?;
    mapping_result?;

    let mapped = slice.get_mapped_range();
    let rgba8 = strip_padded_rows(
        &mapped,
        unpadded_bytes_per_row as usize,
        padded_bytes_per_row as usize,
        viewport.height as usize,
    );
    drop(mapped);
    readback.unmap();

    Ok(RenderedImage {
        width: viewport.width,
        height: viewport.height,
        rgba8,
    })
}

fn resolved_up_vector(camera: &Camera) -> DVec3 {
    let preferred_up = if camera.up.length_squared() <= f64::EPSILON {
        WORLD_UP
    } else {
        camera.up.normalize_or_zero()
    };
    let forward = (camera.target - camera.eye).normalize_or_zero();

    if forward.length_squared() <= f64::EPSILON {
        return preferred_up;
    }

    if forward.cross(preferred_up).length_squared() > 1.0e-6 {
        return preferred_up;
    }

    for fallback_up in [WORLD_FORWARD, WORLD_RIGHT] {
        if forward.cross(fallback_up).length_squared() > 1.0e-6 {
            return fallback_up;
        }
    }

    preferred_up
}

fn dmat4_to_f32_array(matrix: DMat4) -> [[f32; 4]; 4] {
    matrix
        .to_cols_array_2d()
        .map(|column| column.map(|value| value as f32))
}

fn mat4_from_dmat4(matrix: DMat4) -> Mat4 {
    Mat4::from_cols_array_2d(&dmat4_to_f32_array(matrix))
}

fn resolved_light_direction(direction: Vec3) -> Vec3 {
    let direction = direction.normalize_or_zero();

    if direction.length_squared() <= f32::EPSILON {
        Vec3::Z
    } else {
        direction
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn align_to(value: u32, alignment: u32) -> u32 {
    let remainder = value % alignment;
    if remainder == 0 {
        value
    } else {
        value + (alignment - remainder)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn strip_padded_rows(
    data: &[u8],
    unpadded_bytes_per_row: usize,
    padded_bytes_per_row: usize,
    height: usize,
) -> Vec<u8> {
    let mut rgba8 = vec![0; unpadded_bytes_per_row * height];

    for row in 0..height {
        let src_start = row * padded_bytes_per_row;
        let dst_start = row * unpadded_bytes_per_row;
        rgba8[dst_start..dst_start + unpadded_bytes_per_row]
            .copy_from_slice(&data[src_start..src_start + unpadded_bytes_per_row]);
    }

    rgba8
}

#[cfg(test)]
mod tests {
    use super::*;
    use cc_w_types::{
        Bounds3, DisplayColor, GeometryDefinitionId, GeometryInstanceId, PreparedMaterial,
        PreparedRenderDefinition, PreparedRenderInstance, PreparedRenderScene, PreparedVertex,
        WORLD_UP,
    };
    use glam::DVec3;

    #[test]
    fn render_defaults_preserve_current_baseline() {
        let defaults = RenderDefaults::default();

        assert_eq!(defaults.depth_format, wgpu::TextureFormat::Depth32Float);
        assert_eq!(defaults.depth_clear_value, 0.0);
        assert_eq!(defaults.front_face, wgpu::FrontFace::Ccw);
        assert_eq!(defaults.cull_mode, Some(wgpu::Face::Back));
        assert_eq!(defaults.depth_compare, wgpu::CompareFunction::Greater);
        assert_eq!(
            defaults.directional_light.direction,
            Vec3::new(0.35, -0.45, 0.82)
        );
    }

    #[test]
    fn render_classes_control_architectural_edge_overlay_by_edge_kind() {
        let physical = GpuInstance::from_instance(
            DMat4::IDENTITY,
            PreparedMaterial::default(),
            1,
            DefaultRenderClass::Physical,
        );
        let terrain = GpuInstance::from_instance(
            DMat4::IDENTITY,
            PreparedMaterial::default(),
            2,
            DefaultRenderClass::Terrain,
        );
        let terrain_feature = GpuInstance::from_instance(
            DMat4::IDENTITY,
            PreparedMaterial::default(),
            3,
            DefaultRenderClass::TerrainFeature,
        );
        let vegetation = GpuInstance::from_instance(
            DMat4::IDENTITY,
            PreparedMaterial::default(),
            4,
            DefaultRenderClass::Vegetation,
        );
        let vegetation_cover = GpuInstance::from_instance(
            DMat4::IDENTITY,
            PreparedMaterial::default(),
            5,
            DefaultRenderClass::VegetationCover,
        );
        let water = GpuInstance::from_instance(
            DMat4::IDENTITY,
            PreparedMaterial::default(),
            6,
            DefaultRenderClass::Water,
        );
        let surface_decal = GpuInstance::from_instance(
            DMat4::IDENTITY,
            PreparedMaterial::default(),
            7,
            DefaultRenderClass::SurfaceDecal,
        );

        assert_eq!(physical.boundary_edge_visibility, 1.0);
        assert_eq!(physical.crease_edge_visibility, 1.0);
        assert_eq!(physical.outline_index, 1);
        assert_eq!(terrain.boundary_edge_visibility, 0.0);
        assert_eq!(terrain.crease_edge_visibility, 0.0);
        assert_eq!(terrain.outline_index, 0);
        assert_eq!(terrain_feature.boundary_edge_visibility, 0.0);
        assert_eq!(terrain_feature.crease_edge_visibility, 1.0);
        assert_eq!(terrain_feature.outline_index, 3);
        assert_eq!(vegetation.boundary_edge_visibility, 1.0);
        assert_eq!(vegetation.crease_edge_visibility, 0.0);
        assert_eq!(vegetation.outline_index, 4);
        assert_eq!(vegetation_cover.boundary_edge_visibility, 0.0);
        assert_eq!(vegetation_cover.crease_edge_visibility, 0.0);
        assert_eq!(vegetation_cover.outline_index, 0);
        assert_eq!(water.boundary_edge_visibility, 0.0);
        assert_eq!(water.crease_edge_visibility, 0.0);
        assert_eq!(water.outline_index, 0);
        assert_eq!(surface_decal.boundary_edge_visibility, 0.0);
        assert_eq!(surface_decal.crease_edge_visibility, 0.0);
        assert_eq!(surface_decal.outline_index, 0);
    }

    #[test]
    fn reference_grid_vertices_follow_scene_footprint() {
        let bounds = Bounds3 {
            min: DVec3::new(-2.0, -1.0, 0.5),
            max: DVec3::new(4.0, 3.0, 4.0),
        };
        let vertices = reference_grid_vertices(bounds);

        assert!(!vertices.is_empty());
        assert_eq!(vertices.len() % 2, 0);
        assert!(
            vertices
                .iter()
                .all(|vertex| f64::from(vertex.position[2]) < bounds.min.z)
        );
        assert!(vertices.iter().any(|vertex| vertex.alpha > 0.50));
        assert!(vertices.iter().any(|vertex| vertex.alpha < 0.40));
    }

    #[test]
    fn reference_grid_spacing_uses_metric_decades() {
        assert_eq!(metric_reference_grid_spacing(0.006), 0.01);
        assert_eq!(metric_reference_grid_spacing(0.08), 0.1);
        assert_eq!(metric_reference_grid_spacing(0.7), 1.0);
        assert_eq!(metric_reference_grid_spacing(6.0), 10.0);
        assert_eq!(metric_reference_grid_spacing(70.0), 100.0);
    }

    #[test]
    fn architectural_profiles_use_matte_surface_lighting() {
        assert!(!uses_architectural_surface_lighting(
            RenderProfileId::Diffuse
        ));
        assert!(uses_architectural_surface_lighting(
            RenderProfileId::ArchitecturalV1
        ));
        assert!(uses_architectural_surface_lighting(
            RenderProfileId::ArchitecturalV2
        ));
        assert!(uses_architectural_surface_lighting(
            RenderProfileId::ArchitecturalV3
        ));
        assert!(uses_architectural_surface_lighting(
            RenderProfileId::ArchitecturalV4
        ));
    }

    #[test]
    fn upload_summary_matches_prepared_mesh() {
        let mesh = PreparedMesh {
            local_origin: DVec3::ZERO,
            bounds: Bounds3::zero(),
            vertices: vec![
                PreparedVertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                PreparedVertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                PreparedVertex {
                    position: [0.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2],
        };
        let mut backend = NullRenderBackend::default();
        let upload = backend.upload(&mesh);

        assert_eq!(upload.vertex_count, 3);
        assert_eq!(upload.index_count, 3);
        assert_eq!(
            upload.vertex_stride as usize,
            std::mem::size_of::<GpuVertex>()
        );
    }

    #[test]
    fn camera_clip_matrix_changes_with_viewport() {
        let camera = Camera::default();
        let wide = camera.clip_from_world(ViewportSize::new(1280, 720));
        let tall = camera.clip_from_world(ViewportSize::new(720, 1280));

        assert_ne!(wide, tall);
    }

    #[test]
    fn camera_projection_uses_reverse_z_depth() {
        let camera = Camera::default();
        let view_direction = (camera.target - camera.eye).normalize();
        let near_point = camera.eye + view_direction * camera.near_plane;
        let far_point = camera.eye + view_direction * camera.far_plane;
        let clip_from_world = camera.clip_from_world(ViewportSize::new(160, 120));
        let (_, _, near_depth) =
            project_world_point(clip_from_world, ViewportSize::new(160, 120), near_point)
                .expect("near plane projects");
        let (_, _, far_depth) =
            project_world_point(clip_from_world, ViewportSize::new(160, 120), far_point)
                .expect("far plane projects");

        assert!(near_depth > 0.999);
        assert!(far_depth < 0.001);
    }

    #[test]
    fn fit_camera_targets_mesh_center() {
        let mesh = PreparedMesh {
            local_origin: DVec3::ZERO,
            bounds: Bounds3::zero(),
            vertices: vec![
                PreparedVertex {
                    position: [-2.0, -1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                PreparedVertex {
                    position: [2.0, -1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                PreparedVertex {
                    position: [2.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                PreparedVertex {
                    position: [-2.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
        };

        let camera = fit_camera_to_mesh(&mesh);

        assert_eq!(camera.target, DVec3::ZERO);
        assert_eq!(camera.up, WORLD_UP);
        assert!(camera.eye.z > 0.0);
        assert!(camera.eye.y < 0.0);
    }

    #[test]
    fn top_down_z_up_camera_stays_finite() {
        let camera = Camera {
            eye: DVec3::new(0.0, 0.0, 7.0),
            target: DVec3::ZERO,
            up: WORLD_UP,
            ..Camera::default()
        };

        let clip = camera.clip_from_world(ViewportSize::new(160, 120));

        assert!(clip.to_cols_array().into_iter().all(f64::is_finite));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn readback_rows_are_de_padded() {
        let bytes = vec![1, 2, 3, 4, 9, 9, 9, 9, 5, 6, 7, 8, 9, 9, 9, 9];
        let rgba8 = strip_padded_rows(&bytes, 4, 8, 2);

        assert_eq!(rgba8, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn pick_indices_round_trip_through_rgba_bytes() {
        for index in [0, 1, 255, 256, 65_535, 65_536, 0x12_34_56_78] {
            assert_eq!(decode_pick_index(&encode_pick_index(index)), index);
        }
    }

    #[test]
    fn pick_regions_are_clamped_to_viewport() {
        let viewport = ViewportSize::new(100, 80);

        assert_eq!(
            clamp_pick_region(PickRegion::rect(95, 70, 20, 20), viewport),
            Some(PickRegion::rect(95, 70, 5, 10))
        );
        assert_eq!(
            clamp_pick_region(PickRegion::pixel(100, 10), viewport),
            None
        );
    }

    #[test]
    fn cpu_pick_prepared_scene_returns_visible_instances_and_surface_anchor() {
        let mesh = PreparedMesh {
            local_origin: DVec3::ZERO,
            bounds: Bounds3::from_points(&[
                DVec3::new(-0.75, -0.75, 0.0),
                DVec3::new(0.75, 0.75, 0.0),
            ])
            .expect("bounds"),
            vertices: vec![
                PreparedVertex {
                    position: [-0.75, -0.75, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                PreparedVertex {
                    position: [0.75, -0.75, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                PreparedVertex {
                    position: [0.0, 0.75, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2],
        };
        let left_transform = DMat4::from_translation(DVec3::new(-1.0, 0.0, 0.0));
        let right_transform = DMat4::from_translation(DVec3::new(1.0, 0.0, 0.0));
        let scene = PreparedRenderScene {
            bounds: Bounds3::from_points(&[
                DVec3::new(-1.75, -0.75, 0.0),
                DVec3::new(1.75, 0.75, 0.0),
            ])
            .expect("scene bounds"),
            definitions: vec![PreparedRenderDefinition {
                id: GeometryDefinitionId(7),
                mesh: mesh.clone(),
            }],
            instances: vec![
                PreparedRenderInstance {
                    id: GeometryInstanceId(1),
                    element_id: SemanticElementId::new("synthetic/left"),
                    definition_id: GeometryDefinitionId(7),
                    model_from_object: left_transform,
                    world_bounds: mesh.bounds.transformed(left_transform),
                    material: PreparedMaterial::default(),
                    default_render_class: DefaultRenderClass::Physical,
                },
                PreparedRenderInstance {
                    id: GeometryInstanceId(2),
                    element_id: SemanticElementId::new("synthetic/right"),
                    definition_id: GeometryDefinitionId(7),
                    model_from_object: right_transform,
                    world_bounds: mesh.bounds.transformed(right_transform),
                    material: PreparedMaterial::default(),
                    default_render_class: DefaultRenderClass::Physical,
                },
            ],
        };
        let viewport = ViewportSize::new(160, 120);
        let camera = fit_camera_to_render_scene(&scene);

        let rect_result =
            pick_prepared_scene_cpu(&scene, camera, viewport, PickRegion::rect(0, 0, 160, 120));
        let rect_hit_ids = rect_result
            .hits
            .iter()
            .map(|hit| hit.instance_id)
            .collect::<HashSet<_>>();
        assert_eq!(
            rect_hit_ids,
            [GeometryInstanceId(1), GeometryInstanceId(2)]
                .into_iter()
                .collect()
        );

        let left_center = scene.instances[0].world_bounds.center();
        let clip = camera.clip_from_world(viewport)
            * DVec4::new(left_center.x, left_center.y, left_center.z, 1.0);
        let ndc = clip.truncate() / clip.w;
        let x = (((ndc.x + 1.0) * 0.5) * f64::from(viewport.width)).floor() as u32;
        let y = ((1.0 - ((ndc.y + 1.0) * 0.5)) * f64::from(viewport.height)).floor() as u32;
        let point_result =
            pick_prepared_scene_cpu(&scene, camera, viewport, PickRegion::pixel(x, y));

        assert_eq!(point_result.hits.len(), 1);
        assert_eq!(point_result.hits[0].instance_id, GeometryInstanceId(1));
        assert!((point_result.hits[0].world_anchor.z - 0.0).abs() < 1.0e-6);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn scene_upload_batches_instances_per_definition() {
        pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: None,
                })
                .await
                .expect("adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("w render batching test device"),
                    ..Default::default()
                })
                .await
                .expect("device");

            let mesh = PreparedMesh {
                local_origin: DVec3::new(1.0, 2.0, 3.0),
                bounds: Bounds3::from_points(&[
                    DVec3::new(0.0, 1.0, 3.0),
                    DVec3::new(2.0, 3.0, 3.0),
                ])
                .expect("bounds"),
                vertices: vec![
                    PreparedVertex {
                        position: [-1.0, -1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                    PreparedVertex {
                        position: [1.0, -1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                    PreparedVertex {
                        position: [0.0, 1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                ],
                indices: vec![0, 1, 2],
            };
            let scene = PreparedRenderScene {
                bounds: Bounds3::from_points(&[
                    DVec3::new(0.0, 1.0, 3.0),
                    DVec3::new(7.0, 3.0, 3.0),
                ])
                .expect("scene bounds"),
                definitions: vec![PreparedRenderDefinition {
                    id: GeometryDefinitionId(7),
                    mesh: mesh.clone(),
                }],
                instances: vec![
                    PreparedRenderInstance {
                        id: GeometryInstanceId(1),
                        element_id: SemanticElementId::new("synthetic/left"),
                        definition_id: GeometryDefinitionId(7),
                        model_from_object: DMat4::IDENTITY,
                        world_bounds: mesh.bounds,
                        material: PreparedMaterial::default(),
                        default_render_class: DefaultRenderClass::Physical,
                    },
                    PreparedRenderInstance {
                        id: GeometryInstanceId(2),
                        element_id: SemanticElementId::new("synthetic/right"),
                        definition_id: GeometryDefinitionId(7),
                        model_from_object: DMat4::from_translation(DVec3::new(5.0, 0.0, 0.0)),
                        world_bounds: mesh
                            .bounds
                            .transformed(DMat4::from_translation(DVec3::new(5.0, 0.0, 0.0))),
                        material: PreparedMaterial::new(DisplayColor::new(0.9, 0.3, 0.2)),
                        default_render_class: DefaultRenderClass::Physical,
                    },
                    PreparedRenderInstance {
                        id: GeometryInstanceId(3),
                        element_id: SemanticElementId::new("synthetic/marking"),
                        definition_id: GeometryDefinitionId(7),
                        model_from_object: DMat4::from_translation(DVec3::new(2.5, 0.0, 0.0)),
                        world_bounds: mesh
                            .bounds
                            .transformed(DMat4::from_translation(DVec3::new(2.5, 0.0, 0.0))),
                        material: PreparedMaterial::new(DisplayColor::new(1.0, 1.0, 1.0)),
                        default_render_class: DefaultRenderClass::SurfaceDecal,
                    },
                ],
            };

            let mut renderer = MeshRenderer::new(
                &device,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                ViewportSize::new(160, 120),
                fit_camera_to_render_scene(&scene),
            );
            let uploads = renderer.upload_prepared_scene(&device, &queue, &scene);

            assert_eq!(uploads.len(), 1);
            assert!(!renderer.reference_grid_visible());
            renderer.set_reference_grid_visible(true);
            assert!(renderer.reference_grid_visible());
            assert_eq!(renderer.meshes.len(), 1);
            assert_eq!(renderer.instance_batches.len(), 2);
            assert_eq!(renderer.pick_targets.len(), 3);
            assert_eq!(
                renderer.instance_batches[0].render_layer,
                RenderLayer::Opaque
            );
            assert_eq!(renderer.instance_batches[0].instance_count, 2);
            assert_eq!(
                renderer.instance_batches[1].render_layer,
                RenderLayer::SurfaceDecal
            );
            assert_eq!(renderer.instance_batches[1].instance_count, 1);
            assert!(renderer.reference_grid_vertex_buffer.is_some());
            assert!(renderer.reference_grid_vertex_count > 0);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pick_region_returns_visible_unique_instances() {
        pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: None,
                })
                .await
                .expect("adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("w render picking test device"),
                    ..Default::default()
                })
                .await
                .expect("device");

            let mesh = PreparedMesh {
                local_origin: DVec3::ZERO,
                bounds: Bounds3::from_points(&[
                    DVec3::new(-0.75, -0.75, 0.0),
                    DVec3::new(0.75, 0.75, 0.0),
                ])
                .expect("bounds"),
                vertices: vec![
                    PreparedVertex {
                        position: [-0.75, -0.75, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                    PreparedVertex {
                        position: [0.75, -0.75, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                    PreparedVertex {
                        position: [0.0, 0.75, 0.0],
                        normal: [0.0, 0.0, 1.0],
                    },
                ],
                indices: vec![0, 1, 2],
            };
            let left_transform = DMat4::from_translation(DVec3::new(-1.0, 0.0, 0.0));
            let right_transform = DMat4::from_translation(DVec3::new(1.0, 0.0, 0.0));
            let scene = PreparedRenderScene {
                bounds: Bounds3::from_points(&[
                    DVec3::new(-1.75, -0.75, 0.0),
                    DVec3::new(1.75, 0.75, 0.0),
                ])
                .expect("scene bounds"),
                definitions: vec![PreparedRenderDefinition {
                    id: GeometryDefinitionId(7),
                    mesh: mesh.clone(),
                }],
                instances: vec![
                    PreparedRenderInstance {
                        id: GeometryInstanceId(1),
                        element_id: SemanticElementId::new("synthetic/left"),
                        definition_id: GeometryDefinitionId(7),
                        model_from_object: left_transform,
                        world_bounds: mesh.bounds.transformed(left_transform),
                        material: PreparedMaterial::default(),
                        default_render_class: DefaultRenderClass::Physical,
                    },
                    PreparedRenderInstance {
                        id: GeometryInstanceId(2),
                        element_id: SemanticElementId::new("synthetic/right"),
                        definition_id: GeometryDefinitionId(7),
                        model_from_object: right_transform,
                        world_bounds: mesh.bounds.transformed(right_transform),
                        material: PreparedMaterial::new(DisplayColor::new(0.9, 0.3, 0.2)),
                        default_render_class: DefaultRenderClass::Physical,
                    },
                ],
            };

            let mut renderer = MeshRenderer::new(
                &device,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                ViewportSize::new(160, 120),
                fit_camera_to_render_scene(&scene),
            );
            renderer.upload_prepared_scene(&device, &queue, &scene);
            renderer.set_reference_grid_visible(true);

            for profile in [
                RenderProfileId::ArchitecturalV2,
                RenderProfileId::ArchitecturalV3,
                RenderProfileId::ArchitecturalV4,
            ] {
                renderer.set_profile(profile);
                let color_texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("w architectural profile smoke color texture"),
                    size: wgpu::Extent3d {
                        width: 160,
                        height: 120,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
                let depth_target = DepthTarget::with_label(
                    &device,
                    ViewportSize::new(160, 120),
                    "w architectural smoke depth",
                );
                let mut render_encoder =
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("w architectural profile smoke encoder"),
                    });
                renderer.render_with_device(
                    &device,
                    &mut render_encoder,
                    &color_view,
                    depth_target.view(),
                );
                let submission = queue.submit([render_encoder.finish()]);
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: None,
                    })
                    .expect("architectural profile render completes");
            }

            let result = renderer
                .pick_region(&device, &queue, PickRegion::rect(0, 0, 160, 120))
                .expect("pick result");
            let hit_ids = result
                .hits
                .iter()
                .map(|hit| hit.instance_id)
                .collect::<std::collections::HashSet<_>>();

            assert_eq!(
                hit_ids,
                [GeometryInstanceId(1), GeometryInstanceId(2)]
                    .into_iter()
                    .collect()
            );
        });
    }
}
