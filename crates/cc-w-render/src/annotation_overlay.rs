use bytemuck::{Pod, Zeroable, cast_slice};
use cc_w_types::{
    SceneAnnotationDepthMode, SceneAnnotationLayer, SceneAnnotationPrimitive, SceneMarkerKind,
    SceneTextLabel,
};
use glam::DVec3;
use wgpu::util::DeviceExt;
use wgpu::vertex_attr_array;

use crate::{RenderDefaults, depth_compare_equal_variant};

const ANNOTATION_POLYLINE_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
    viewport_and_profile : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

struct VertexInput {
    @location(0) start_position : vec3<f32>,
    @location(1) end_position : vec3<f32>,
    @location(2) endpoint_side_cap : vec3<f32>,
    @location(3) width_px : f32,
    @location(4) color : vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) @interpolate(flat) start_px : vec2<f32>,
    @location(1) @interpolate(flat) end_px : vec2<f32>,
    @location(2) @interpolate(flat) width_alpha : vec2<f32>,
    @location(3) @interpolate(flat) color : vec4<f32>,
};

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    let viewport = max(camera.viewport_and_profile.xy, vec2<f32>(1.0, 1.0));
    let start_clip = camera.clip_from_world * vec4<f32>(input.start_position, 1.0);
    let end_clip = camera.clip_from_world * vec4<f32>(input.end_position, 1.0);
    let start_px = ((start_clip.xy / start_clip.w) * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5)) * viewport;
    let end_px = ((end_clip.xy / end_clip.w) * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5)) * viewport;
    let delta_px = end_px - start_px;
    let delta_length = length(delta_px);
    let direction = select(vec2<f32>(1.0, 0.0), delta_px / delta_length, delta_length > 0.0001);
    let normal = vec2<f32>(-direction.y, direction.x);
    let half_width = max(input.width_px * 0.5, 0.5);
    let t = clamp(input.endpoint_side_cap.x, 0.0, 1.0);
    let cap_offset = input.endpoint_side_cap.z * half_width;
    let pixel_position = mix(start_px, end_px, t)
        + direction * cap_offset
        + normal * input.endpoint_side_cap.y * half_width;
    let ndc = (pixel_position / viewport - vec2<f32>(0.5, 0.5)) * vec2<f32>(2.0, -2.0);
    let base_clip = mix(start_clip, end_clip, t);

    var out : VertexOutput;
    out.position = vec4<f32>(ndc * base_clip.w, base_clip.z, base_clip.w);
    out.start_px = start_px;
    out.end_px = end_px;
    out.width_alpha = vec2<f32>(half_width, input.color.a);
    out.color = input.color;
    return out;
}

fn segment_distance(point : vec2<f32>, start : vec2<f32>, end : vec2<f32>) -> f32 {
    let segment = end - start;
    let t = clamp(dot(point - start, segment) / max(dot(segment, segment), 0.0001), 0.0, 1.0);
    return length(point - (start + segment * t));
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    let distance_px = segment_distance(input.position.xy, input.start_px, input.end_px);
    let edge_alpha = 1.0 - smoothstep(input.width_alpha.x - 0.5, input.width_alpha.x + 0.5, distance_px);
    if (edge_alpha <= 0.001) {
        discard;
    }
    return vec4<f32>(input.color.rgb, input.width_alpha.y * edge_alpha);
}
"#;

const ANNOTATION_MARKER_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
    viewport_and_profile : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

struct VertexInput {
    @location(0) position : vec3<f32>,
    @location(1) offset_px : vec2<f32>,
    @location(2) local : vec2<f32>,
    @location(3) kind : f32,
    @location(4) color : vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) local : vec2<f32>,
    @location(1) kind : f32,
    @location(2) color : vec4<f32>,
};

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    var out : VertexOutput;
    let viewport = max(camera.viewport_and_profile.xy, vec2<f32>(1.0, 1.0));
    let clip = camera.clip_from_world * vec4<f32>(input.position, 1.0);
    let offset_ndc = input.offset_px / viewport * vec2<f32>(2.0, -2.0);
    out.position = clip + vec4<f32>(offset_ndc * clip.w, 0.0, 0.0);
    out.local = input.local;
    out.kind = input.kind;
    out.color = input.color;
    return out;
}

fn segment_distance(point : vec2<f32>, start : vec2<f32>, end : vec2<f32>) -> f32 {
    let segment = end - start;
    let t = clamp(dot(point - start, segment) / max(dot(segment, segment), 0.0001), 0.0, 1.0);
    return length(point - (start + segment * t));
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    let local = input.local;
    var alpha = 0.0;
    if (input.kind < 0.5) {
        alpha = 1.0 - smoothstep(0.82, 1.0, length(local));
    } else if (input.kind < 1.5) {
        let bar = min(abs(local.x), abs(local.y));
        alpha = select(0.0, 1.0, bar <= 0.18 && max(abs(local.x), abs(local.y)) <= 0.96);
    } else if (input.kind < 2.5) {
        let d0 = segment_distance(local, vec2<f32>(-0.72, -0.02), vec2<f32>(-0.20, 0.56));
        let d1 = segment_distance(local, vec2<f32>(-0.20, 0.56), vec2<f32>(0.78, -0.62));
        alpha = select(0.0, 1.0, min(d0, d1) <= 0.16);
    } else {
        let inside_head = abs(local.x) <= (1.0 - local.y) * 0.45 && local.y <= 0.82 && local.y >= -0.18;
        let inside_tail = abs(local.x) <= 0.16 && local.y >= -0.92 && local.y < -0.10;
        alpha = select(0.0, 1.0, inside_head || inside_tail);
    }
    if (alpha <= 0.001) {
        discard;
    }
    return vec4<f32>(input.color.rgb, input.color.a * alpha);
}
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnnotationDepthBucket {
    DepthTested,
    DrawThrough,
}

impl AnnotationDepthBucket {
    fn from_mode(mode: SceneAnnotationDepthMode) -> Self {
        match mode {
            SceneAnnotationDepthMode::DepthTested => Self::DepthTested,
            SceneAnnotationDepthMode::Overlay | SceneAnnotationDepthMode::XRay => Self::DrawThrough,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct GpuAnnotationPolylineVertex {
    start_position: [f32; 3],
    end_position: [f32; 3],
    endpoint_side_cap: [f32; 3],
    width_px: f32,
    color: [f32; 4],
}

impl GpuAnnotationPolylineVertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 5] = vertex_attr_array![
            0 => Float32x3,
            1 => Float32x3,
            2 => Float32x3,
            3 => Float32,
            4 => Float32x4
        ];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuAnnotationPolylineVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct GpuAnnotationMarkerVertex {
    position: [f32; 3],
    offset_px: [f32; 2],
    local: [f32; 2],
    kind: f32,
    color: [f32; 4],
}

impl GpuAnnotationMarkerVertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 5] = vertex_attr_array![
            0 => Float32x3,
            1 => Float32x2,
            2 => Float32x2,
            3 => Float32,
            4 => Float32x4
        ];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuAnnotationMarkerVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AnnotationOverlayGeometry {
    pub layer_count: u32,
    pub primitive_count: u32,
    pub polyline_depth_tested_vertices: Vec<GpuAnnotationPolylineVertex>,
    pub polyline_draw_through_vertices: Vec<GpuAnnotationPolylineVertex>,
    pub marker_depth_tested_vertices: Vec<GpuAnnotationMarkerVertex>,
    pub marker_draw_through_vertices: Vec<GpuAnnotationMarkerVertex>,
    pub text_labels: Vec<SceneTextLabel>,
}

#[derive(Debug, Default)]
pub(crate) struct AnnotationOverlayGpuState {
    pub layer_count: u32,
    pub primitive_count: u32,
    pub polyline_depth_tested_vertex_buffer: Option<wgpu::Buffer>,
    pub polyline_depth_tested_vertex_count: u32,
    pub polyline_draw_through_vertex_buffer: Option<wgpu::Buffer>,
    pub polyline_draw_through_vertex_count: u32,
    pub marker_depth_tested_vertex_buffer: Option<wgpu::Buffer>,
    pub marker_depth_tested_vertex_count: u32,
    pub marker_draw_through_vertex_buffer: Option<wgpu::Buffer>,
    pub marker_draw_through_vertex_count: u32,
    pub text_labels: Vec<SceneTextLabel>,
}

impl AnnotationOverlayGpuState {
    pub fn from_geometry(device: &wgpu::Device, geometry: AnnotationOverlayGeometry) -> Self {
        Self {
            layer_count: geometry.layer_count,
            primitive_count: geometry.primitive_count,
            polyline_depth_tested_vertex_count: geometry.polyline_depth_tested_vertices.len()
                as u32,
            polyline_depth_tested_vertex_buffer: vertex_buffer(
                device,
                "w annotation depth-tested polyline vertex buffer",
                &geometry.polyline_depth_tested_vertices,
            ),
            polyline_draw_through_vertex_count: geometry.polyline_draw_through_vertices.len()
                as u32,
            polyline_draw_through_vertex_buffer: vertex_buffer(
                device,
                "w annotation draw-through polyline vertex buffer",
                &geometry.polyline_draw_through_vertices,
            ),
            marker_depth_tested_vertex_count: geometry.marker_depth_tested_vertices.len() as u32,
            marker_depth_tested_vertex_buffer: vertex_buffer(
                device,
                "w annotation depth-tested marker vertex buffer",
                &geometry.marker_depth_tested_vertices,
            ),
            marker_draw_through_vertex_count: geometry.marker_draw_through_vertices.len() as u32,
            marker_draw_through_vertex_buffer: vertex_buffer(
                device,
                "w annotation draw-through marker vertex buffer",
                &geometry.marker_draw_through_vertices,
            ),
            text_labels: geometry.text_labels,
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn has_vertices(&self) -> bool {
        self.polyline_depth_tested_vertex_count > 0
            || self.polyline_draw_through_vertex_count > 0
            || self.marker_depth_tested_vertex_count > 0
            || self.marker_draw_through_vertex_count > 0
    }
}

#[derive(Debug)]
pub(crate) struct AnnotationOverlayPipelines {
    polyline_depth_tested_pipeline: wgpu::RenderPipeline,
    polyline_draw_through_pipeline: wgpu::RenderPipeline,
    marker_depth_tested_pipeline: wgpu::RenderPipeline,
    marker_draw_through_pipeline: wgpu::RenderPipeline,
}

impl AnnotationOverlayPipelines {
    pub fn new(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        color_format: wgpu::TextureFormat,
        defaults: RenderDefaults,
    ) -> Self {
        let polyline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w annotation polyline shader"),
            source: wgpu::ShaderSource::Wgsl(ANNOTATION_POLYLINE_SHADER_WGSL.into()),
        });
        let marker_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w annotation marker shader"),
            source: wgpu::ShaderSource::Wgsl(ANNOTATION_MARKER_SHADER_WGSL.into()),
        });

        Self {
            polyline_depth_tested_pipeline: create_annotation_pipeline(
                device,
                "w annotation depth-tested polyline pipeline",
                pipeline_layout,
                &polyline_shader,
                &[GpuAnnotationPolylineVertex::layout()],
                color_format,
                defaults,
                wgpu::CompareFunction::LessEqual,
            ),
            polyline_draw_through_pipeline: create_annotation_pipeline(
                device,
                "w annotation draw-through polyline pipeline",
                pipeline_layout,
                &polyline_shader,
                &[GpuAnnotationPolylineVertex::layout()],
                color_format,
                defaults,
                wgpu::CompareFunction::Always,
            ),
            marker_depth_tested_pipeline: create_annotation_pipeline(
                device,
                "w annotation depth-tested marker pipeline",
                pipeline_layout,
                &marker_shader,
                &[GpuAnnotationMarkerVertex::layout()],
                color_format,
                defaults,
                wgpu::CompareFunction::LessEqual,
            ),
            marker_draw_through_pipeline: create_annotation_pipeline(
                device,
                "w annotation draw-through marker pipeline",
                pipeline_layout,
                &marker_shader,
                &[GpuAnnotationMarkerVertex::layout()],
                color_format,
                defaults,
                wgpu::CompareFunction::Always,
            ),
        }
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
        scene_bind_group: &wgpu::BindGroup,
        state: &AnnotationOverlayGpuState,
    ) {
        if !state.has_vertices() {
            return;
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("w annotation overlay pass"),
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
        pass.set_bind_group(0, scene_bind_group, &[]);

        draw_buffer(
            &mut pass,
            &self.polyline_depth_tested_pipeline,
            &state.polyline_depth_tested_vertex_buffer,
            state.polyline_depth_tested_vertex_count,
        );
        draw_buffer(
            &mut pass,
            &self.marker_depth_tested_pipeline,
            &state.marker_depth_tested_vertex_buffer,
            state.marker_depth_tested_vertex_count,
        );
        draw_buffer(
            &mut pass,
            &self.polyline_draw_through_pipeline,
            &state.polyline_draw_through_vertex_buffer,
            state.polyline_draw_through_vertex_count,
        );
        draw_buffer(
            &mut pass,
            &self.marker_draw_through_pipeline,
            &state.marker_draw_through_vertex_buffer,
            state.marker_draw_through_vertex_count,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AnnotationOverlayError {
    #[error(
        "annotation layer {layer_index} primitive {primitive_index} point {point_index} is not finite"
    )]
    NonFinitePoint {
        layer_index: usize,
        primitive_index: usize,
        point_index: usize,
    },
    #[error("annotation layer {layer_index} primitive {primitive_index} position is not finite")]
    NonFinitePosition {
        layer_index: usize,
        primitive_index: usize,
    },
    #[error(
        "annotation layer {layer_index} primitive {primitive_index} color component {component_index} is not finite"
    )]
    NonFiniteColor {
        layer_index: usize,
        primitive_index: usize,
        component_index: usize,
    },
    #[error("annotation layer {layer_index} primitive {primitive_index} alpha is not finite")]
    NonFiniteAlpha {
        layer_index: usize,
        primitive_index: usize,
    },
    #[error("annotation layer {layer_index} primitive {primitive_index} size is not finite")]
    NonFiniteSize {
        layer_index: usize,
        primitive_index: usize,
    },
    #[error("annotation text label {label_index} anchor is not finite")]
    NonFiniteTextAnchor { label_index: usize },
    #[error("annotation text label {label_index} screen offset is not finite")]
    NonFiniteTextOffset { label_index: usize },
    #[error("annotation text label {label_index} size is not finite or positive")]
    InvalidTextSize { label_index: usize },
    #[error("annotation text label {label_index} could not be laid out")]
    TextLayout { label_index: usize },
}

pub(crate) fn annotation_overlay_geometry(
    layers: &[SceneAnnotationLayer],
) -> Result<AnnotationOverlayGeometry, AnnotationOverlayError> {
    let mut geometry = AnnotationOverlayGeometry::default();

    for (layer_index, layer) in layers.iter().enumerate() {
        if !layer.visible {
            continue;
        }
        geometry.layer_count += 1;
        geometry.primitive_count += layer.primitives.len() as u32;

        for (primitive_index, primitive) in layer.primitives.iter().enumerate() {
            match primitive {
                SceneAnnotationPrimitive::Polyline(polyline) => {
                    let color = annotation_color(
                        polyline.color.as_rgb(),
                        polyline.alpha,
                        layer_index,
                        primitive_index,
                    )?;
                    if !polyline.width_px.is_finite() {
                        return Err(AnnotationOverlayError::NonFiniteSize {
                            layer_index,
                            primitive_index,
                        });
                    }
                    if polyline.width_px <= 0.0 {
                        continue;
                    }
                    for (point_index, point) in polyline.points.iter().enumerate() {
                        if !point.is_finite() {
                            return Err(AnnotationOverlayError::NonFinitePoint {
                                layer_index,
                                primitive_index,
                                point_index,
                            });
                        }
                    }
                    let bucket = AnnotationDepthBucket::from_mode(polyline.depth_mode);
                    let vertices = match bucket {
                        AnnotationDepthBucket::DepthTested => {
                            &mut geometry.polyline_depth_tested_vertices
                        }
                        AnnotationDepthBucket::DrawThrough => {
                            &mut geometry.polyline_draw_through_vertices
                        }
                    };
                    for segment in polyline.points.windows(2) {
                        push_polyline_segment(
                            vertices,
                            segment[0],
                            segment[1],
                            polyline.width_px,
                            color,
                        );
                    }
                }
                SceneAnnotationPrimitive::Marker(marker) => {
                    if !marker.position.is_finite() {
                        return Err(AnnotationOverlayError::NonFinitePosition {
                            layer_index,
                            primitive_index,
                        });
                    }
                    let color = annotation_color(
                        marker.color.as_rgb(),
                        marker.alpha,
                        layer_index,
                        primitive_index,
                    )?;
                    if !marker.size_px.is_finite() {
                        return Err(AnnotationOverlayError::NonFiniteSize {
                            layer_index,
                            primitive_index,
                        });
                    }
                    if marker.size_px <= 0.0 {
                        continue;
                    }
                    let bucket = AnnotationDepthBucket::from_mode(marker.depth_mode);
                    let vertices = match bucket {
                        AnnotationDepthBucket::DepthTested => {
                            &mut geometry.marker_depth_tested_vertices
                        }
                        AnnotationDepthBucket::DrawThrough => {
                            &mut geometry.marker_draw_through_vertices
                        }
                    };
                    push_marker(
                        vertices,
                        marker.position,
                        marker.size_px,
                        marker.kind,
                        color,
                    );
                }
                SceneAnnotationPrimitive::Text(label) => geometry.text_labels.push(label.clone()),
            }
        }
    }

    Ok(geometry)
}

fn create_annotation_pipeline(
    device: &wgpu::Device,
    label: &'static str,
    pipeline_layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    buffers: &[wgpu::VertexBufferLayout<'static>],
    color_format: wgpu::TextureFormat,
    defaults: RenderDefaults,
    depth_compare: wgpu::CompareFunction,
) -> wgpu::RenderPipeline {
    let depth_compare = if depth_compare == wgpu::CompareFunction::Always {
        wgpu::CompareFunction::Always
    } else {
        depth_compare_equal_variant(defaults.depth_compare)
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers,
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
            depth_compare: Some(depth_compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
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
    })
}

fn draw_buffer<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    pipeline: &'pass wgpu::RenderPipeline,
    vertex_buffer: &'pass Option<wgpu::Buffer>,
    vertex_count: u32,
) {
    let Some(vertex_buffer) = vertex_buffer else {
        return;
    };
    if vertex_count == 0 {
        return;
    }

    pass.set_pipeline(pipeline);
    pass.set_vertex_buffer(0, vertex_buffer.slice(..));
    pass.draw(0..vertex_count, 0..1);
}

fn vertex_buffer<T: Pod>(
    device: &wgpu::Device,
    label: &'static str,
    vertices: &[T],
) -> Option<wgpu::Buffer> {
    (!vertices.is_empty()).then(|| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        })
    })
}

fn annotation_color(
    rgb: [f32; 3],
    alpha: f32,
    layer_index: usize,
    primitive_index: usize,
) -> Result<[f32; 4], AnnotationOverlayError> {
    for (component_index, component) in rgb.into_iter().enumerate() {
        if !component.is_finite() {
            return Err(AnnotationOverlayError::NonFiniteColor {
                layer_index,
                primitive_index,
                component_index,
            });
        }
    }
    if !alpha.is_finite() {
        return Err(AnnotationOverlayError::NonFiniteAlpha {
            layer_index,
            primitive_index,
        });
    }
    Ok([rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0)])
}

fn push_polyline_segment(
    vertices: &mut Vec<GpuAnnotationPolylineVertex>,
    start: DVec3,
    end: DVec3,
    width_px: f32,
    color: [f32; 4],
) {
    if start.distance_squared(end) <= f64::EPSILON {
        return;
    }
    let start = [start.x as f32, start.y as f32, start.z as f32];
    let end = [end.x as f32, end.y as f32, end.z as f32];
    let width_px = width_px.max(1.0);
    for endpoint_side_cap in [
        [0.0, -1.0, -1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, -1.0, -1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, -1.0],
    ] {
        vertices.push(GpuAnnotationPolylineVertex {
            start_position: start,
            end_position: end,
            endpoint_side_cap,
            width_px,
            color,
        });
    }
}

fn push_marker(
    vertices: &mut Vec<GpuAnnotationMarkerVertex>,
    position: DVec3,
    size_px: f32,
    kind: SceneMarkerKind,
    color: [f32; 4],
) {
    let position = [position.x as f32, position.y as f32, position.z as f32];
    let half_size = (size_px * 0.5).max(1.0);
    let kind = match kind {
        SceneMarkerKind::Dot => 0.0,
        SceneMarkerKind::Cross => 1.0,
        SceneMarkerKind::Tick => 2.0,
        SceneMarkerKind::Arrow => 3.0,
    };
    for local in [
        [-1.0, -1.0],
        [1.0, -1.0],
        [1.0, 1.0],
        [-1.0, -1.0],
        [1.0, 1.0],
        [-1.0, 1.0],
    ] {
        vertices.push(GpuAnnotationMarkerVertex {
            position,
            offset_px: [local[0] * half_size, local[1] * half_size],
            local,
            kind,
            color,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cc_w_types::{
        DisplayColor, SceneAnnotationLayer, SceneAnnotationPrimitive, SceneMarker, ScenePolyline,
    };

    #[test]
    fn annotation_geometry_splits_depth_modes_and_keeps_text_state() {
        let mut depth_line =
            ScenePolyline::new("depth-line", vec![DVec3::ZERO, DVec3::new(1.0, 0.0, 0.0)]);
        depth_line.depth_mode = SceneAnnotationDepthMode::DepthTested;
        depth_line.color = DisplayColor::new(1.0, 0.0, 0.0);
        depth_line.alpha = 0.5;

        let mut overlay_marker = SceneMarker::new("overlay-marker", DVec3::new(0.0, 1.0, 0.0));
        overlay_marker.depth_mode = SceneAnnotationDepthMode::Overlay;
        overlay_marker.kind = SceneMarkerKind::Cross;

        let text = SceneTextLabel::new("label", "A", DVec3::new(0.0, 0.0, 1.0));
        let mut layer = SceneAnnotationLayer::new("measurements");
        layer.primitives = vec![
            SceneAnnotationPrimitive::Polyline(depth_line),
            SceneAnnotationPrimitive::Marker(overlay_marker),
            SceneAnnotationPrimitive::Text(text),
        ];

        let geometry = annotation_overlay_geometry(&[layer]).expect("valid annotation geometry");

        assert_eq!(geometry.layer_count, 1);
        assert_eq!(geometry.primitive_count, 3);
        assert_eq!(geometry.polyline_depth_tested_vertices.len(), 6);
        assert!(geometry.polyline_draw_through_vertices.is_empty());
        assert!(geometry.marker_depth_tested_vertices.is_empty());
        assert_eq!(geometry.marker_draw_through_vertices.len(), 6);
        assert_eq!(geometry.text_labels.len(), 1);
        assert_eq!(geometry.polyline_depth_tested_vertices[0].color[3], 0.5);
    }

    #[test]
    fn annotation_geometry_rejects_non_finite_positions() {
        let mut layer = SceneAnnotationLayer::new("diagnostic");
        layer
            .primitives
            .push(SceneAnnotationPrimitive::Marker(SceneMarker::new(
                "bad",
                DVec3::new(f64::NAN, 0.0, 0.0),
            )));

        assert_eq!(
            annotation_overlay_geometry(&[layer]).unwrap_err(),
            AnnotationOverlayError::NonFinitePosition {
                layer_index: 0,
                primitive_index: 0,
            }
        );
    }
}
