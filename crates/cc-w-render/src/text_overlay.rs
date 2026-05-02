use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};
use cc_w_types::{
    SceneTextDepthMode, SceneTextHorizontalAlign, SceneTextLabel, SceneTextVerticalAlign,
};
use glam::{DMat4, DVec2, DVec3, DVec4};
use wgpu::util::DeviceExt;
use wgpu::vertex_attr_array;

pub const TEXT_OVERLAY_SHADER_WGSL: &str = r#"
struct Camera {
    clip_from_world : mat4x4<f32>,
    viewport_and_profile : vec4<f32>,
    view_from_world : mat4x4<f32>,
    clip_plane : vec4<f32>,
    clip_params : vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera : Camera;

@group(1) @binding(0)
var text_atlas : texture_2d<f32>;

@group(1) @binding(1)
var text_sampler : sampler;

struct GlyphInstance {
    anchor_world_depth : vec4<f32>,
    rect_px : vec4<f32>,
    uv_rect : vec4<f32>,
    color : vec4<f32>,
    outline_color : vec4<f32>,
    sdf_params : vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) uv : vec2<f32>,
    @location(1) color : vec4<f32>,
    @location(2) outline_color : vec4<f32>,
    @location(3) sdf_params : vec4<f32>,
    @location(4) uv_rect : vec4<f32>,
};

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index : u32,
    @location(0) anchor_world_depth : vec4<f32>,
    @location(1) rect_px : vec4<f32>,
    @location(2) uv_rect : vec4<f32>,
    @location(3) color : vec4<f32>,
    @location(4) outline_color : vec4<f32>,
    @location(5) sdf_params : vec4<f32>,
) -> VertexOutput {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex_index];

    let anchor_clip = camera.clip_from_world * vec4<f32>(anchor_world_depth.xyz, 1.0);
    let viewport = max(camera.viewport_and_profile.xy, vec2<f32>(1.0, 1.0));
    let anchor_ndc = anchor_clip.xy / anchor_clip.w;
    let anchor_px = (anchor_ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5)) * viewport;
    let glyph_min_px = round(anchor_px + rect_px.xy);
    let glyph_max_px = round(anchor_px + rect_px.xy + rect_px.zw);
    let glyph_px = mix(glyph_min_px, glyph_max_px, corner);
    let glyph_ndc = (glyph_px / viewport - vec2<f32>(0.5, 0.5)) * vec2<f32>(2.0, -2.0);

    var out : VertexOutput;
    out.position = vec4<f32>(glyph_ndc * anchor_clip.w, anchor_clip.z, anchor_clip.w);
    out.uv = uv_rect.xy + corner * uv_rect.zw;
    out.color = color;
    out.outline_color = outline_color;
    out.sdf_params = sdf_params;
    out.uv_rect = uv_rect;
    return out;
}

fn atlas_alpha(input : VertexOutput, pixel_offset : vec2<i32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(text_atlas));
    let texel_size = 1.0 / vec2<f32>(max(dims, vec2<i32>(1, 1)));
    let sample_uv = input.uv + vec2<f32>(pixel_offset) * texel_size;
    let uv_min = input.uv_rect.xy;
    let uv_max = input.uv_rect.xy + input.uv_rect.zw;
    if (sample_uv.x < uv_min.x || sample_uv.y < uv_min.y || sample_uv.x >= uv_max.x || sample_uv.y >= uv_max.y) {
        return 0.0;
    }
    let sample_texel = clamp(
        vec2<i32>(floor(sample_uv * vec2<f32>(dims))),
        vec2<i32>(0, 0),
        dims - vec2<i32>(1, 1),
    );
    return textureLoad(text_atlas, sample_texel, 0).r;
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    let fill_threshold = clamp(0.5 - max(input.sdf_params.w, 0.0) * 0.25, 0.05, 0.95);
    let center_alpha = atlas_alpha(input, vec2<i32>(0, 0));
    let fill_alpha = select(0.0, 1.0, center_alpha >= fill_threshold);
    let outline_radius = min(max(input.sdf_params.y, 0.0), 4.0);
    var outline_alpha = fill_alpha;
    for (var y = -4; y <= 4; y = y + 1) {
        for (var x = -4; x <= 4; x = x + 1) {
            let offset = vec2<i32>(x, y);
            let distance_px = length(vec2<f32>(offset));
            if (distance_px <= outline_radius && atlas_alpha(input, offset) >= fill_threshold) {
                outline_alpha = 1.0;
            }
        }
    }
    let outline_only_alpha = max(outline_alpha - fill_alpha, 0.0);
    let rgb = mix(input.outline_color.rgb, input.color.rgb, fill_alpha);
    let alpha = input.color.a * fill_alpha + input.outline_color.a * outline_only_alpha;
    if (alpha <= input.sdf_params.z) {
        discard;
    }
    return vec4<f32>(rgb, alpha);
}
"#;

pub const TEXT_GLYPH_VERTICES_PER_INSTANCE: u32 = 6;
#[cfg(test)]
pub const DEFAULT_SDF_PIXEL_RANGE: f32 = 4.0;
pub const DEFAULT_ALPHA_DISCARD_THRESHOLD: f32 = 0.005;
pub const EMPTY_ATLAS_TEXEL: u8 = 0;
const TEXT_ATLAS_FILTER_MODE: wgpu::FilterMode = wgpu::FilterMode::Nearest;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuTextGlyphInstance {
    pub anchor_world_depth: [f32; 4],
    pub rect_px: [f32; 4],
    pub uv_rect: [f32; 4],
    pub color: [f32; 4],
    pub outline_color: [f32; 4],
    pub sdf_params: [f32; 4],
}

impl GpuTextGlyphInstance {
    pub fn new(
        anchor_world: [f32; 3],
        depth_mode: SceneTextDepthMode,
        rect_px: [f32; 4],
        uv_rect: [f32; 4],
        color: [f32; 4],
        outline_color: [f32; 4],
        sdf_pixel_range: f32,
        outline_width_px: f32,
        embolden_px: f32,
    ) -> Self {
        Self {
            anchor_world_depth: [
                anchor_world[0],
                anchor_world[1],
                anchor_world[2],
                depth_mode_code(depth_mode),
            ],
            rect_px,
            uv_rect,
            color: sanitize_color(color),
            outline_color: sanitize_color(outline_color),
            sdf_params: [
                sdf_pixel_range.max(0.0001),
                outline_width_px.max(0.0),
                DEFAULT_ALPHA_DISCARD_THRESHOLD,
                embolden_px.max(0.0),
            ],
        }
    }

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 6] = vertex_attr_array![
            0 => Float32x4,
            1 => Float32x4,
            2 => Float32x4,
            3 => Float32x4,
            4 => Float32x4,
            5 => Float32x4
        ];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuTextGlyphInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRIBUTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextGlyph {
    pub anchor_world: DVec3,
    pub rect_px: [f32; 4],
    pub uv_rect: [f32; 4],
    pub color: [f32; 4],
    pub outline_color: [f32; 4],
    pub sdf_pixel_range: f32,
    pub outline_width_px: f32,
    pub embolden_px: f32,
    pub depth_mode: SceneTextDepthMode,
}

impl TextGlyph {
    pub fn to_gpu(self) -> Option<GpuTextGlyphInstance> {
        if !self.anchor_world.is_finite()
            || !all_finite(&self.rect_px)
            || !all_finite(&self.uv_rect)
        {
            return None;
        }
        Some(GpuTextGlyphInstance::new(
            [
                self.anchor_world.x as f32,
                self.anchor_world.y as f32,
                self.anchor_world.z as f32,
            ],
            self.depth_mode,
            self.rect_px,
            self.uv_rect,
            self.color,
            self.outline_color,
            self.sdf_pixel_range,
            self.outline_width_px,
            self.embolden_px,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextAtlasDescriptor {
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
}

impl TextAtlasDescriptor {
    pub fn sdf_r8(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            format: wgpu::TextureFormat::R8Unorm,
        }
    }

    pub fn extent(self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width.max(1),
            height: self.height.max(1),
            depth_or_array_layers: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextAtlasRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl TextAtlasRegion {
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    pub fn byte_len(self, bytes_per_texel: u32) -> Option<usize> {
        self.width
            .checked_mul(self.height)?
            .checked_mul(bytes_per_texel)?
            .try_into()
            .ok()
    }
}

pub struct TextAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    descriptor: TextAtlasDescriptor,
}

impl TextAtlas {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        descriptor: TextAtlasDescriptor,
        data: Option<&[u8]>,
    ) -> Self {
        let descriptor = TextAtlasDescriptor {
            width: descriptor.width.max(1),
            height: descriptor.height.max(1),
            format: descriptor.format,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("w text overlay atlas"),
            size: descriptor.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: descriptor.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[descriptor.format],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("w text overlay atlas sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: TEXT_ATLAS_FILTER_MODE,
            min_filter: TEXT_ATLAS_FILTER_MODE,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind_group = create_text_atlas_bind_group(device, layout, &view, &sampler);
        let atlas = Self {
            texture,
            view,
            sampler,
            bind_group,
            descriptor,
        };

        if let Some(data) = data {
            atlas.update(
                queue,
                TextAtlasRegion::full(descriptor.width, descriptor.height),
                data,
            );
        } else {
            let Some(byte_len) = TextAtlasRegion::full(descriptor.width, descriptor.height)
                .byte_len(bytes_per_texel(descriptor.format))
            else {
                return atlas;
            };
            let empty = vec![EMPTY_ATLAS_TEXEL; byte_len];
            atlas.update(
                queue,
                TextAtlasRegion::full(descriptor.width, descriptor.height),
                &empty,
            );
        }

        atlas
    }

    pub fn update(&self, queue: &wgpu::Queue, region: TextAtlasRegion, data: &[u8]) {
        let bytes_per_texel = bytes_per_texel(self.descriptor.format);
        if region.width == 0 || region.height == 0 || bytes_per_texel == 0 {
            return;
        }
        let expected_len = region.byte_len(bytes_per_texel).unwrap_or(usize::MAX);
        assert!(
            data.len() >= expected_len,
            "text atlas update needs at least {expected_len} bytes, got {}",
            data.len()
        );
        assert!(
            region.x + region.width <= self.descriptor.width
                && region.y + region.height <= self.descriptor.height,
            "text atlas update region must fit inside the atlas"
        );

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: region.x,
                    y: region.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &data[..expected_len],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(region.width * bytes_per_texel),
                rows_per_image: Some(region.height),
            },
            wgpu::Extent3d {
                width: region.width,
                height: region.height,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    pub fn descriptor(&self) -> TextAtlasDescriptor {
        self.descriptor
    }
}

pub struct TextOverlayPipeline {
    pipeline: wgpu::RenderPipeline,
    atlas_bind_group_layout: wgpu::BindGroupLayout,
}

impl TextOverlayPipeline {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: Option<wgpu::TextureFormat>,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        depth_mode: SceneTextDepthMode,
    ) -> Self {
        let atlas_bind_group_layout = create_text_atlas_bind_group_layout(device);
        Self::with_atlas_bind_group_layout(
            device,
            color_format,
            depth_format,
            camera_bind_group_layout,
            &atlas_bind_group_layout,
            depth_mode,
        )
    }

    pub fn with_atlas_bind_group_layout(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: Option<wgpu::TextureFormat>,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        atlas_bind_group_layout: &wgpu::BindGroupLayout,
        depth_mode: SceneTextDepthMode,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w text overlay shader"),
            source: wgpu::ShaderSource::Wgsl(TEXT_OVERLAY_SHADER_WGSL.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("w text overlay pipeline layout"),
            bind_group_layouts: &[
                Some(camera_bind_group_layout),
                Some(atlas_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w text overlay pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuTextGlyphInstance::layout()],
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
            depth_stencil: depth_format.map(|format| wgpu::DepthStencilState {
                format,
                depth_write_enabled: Some(false),
                depth_compare: Some(depth_compare_for_text_mode(depth_mode)),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            atlas_bind_group_layout: atlas_bind_group_layout.clone(),
        }
    }

    pub fn atlas_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.atlas_bind_group_layout
    }

    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }
}

pub struct TextOverlayGlyphBuffer {
    buffer: Option<wgpu::Buffer>,
    glyph_count: u32,
}

impl TextOverlayGlyphBuffer {
    pub fn empty() -> Self {
        Self {
            buffer: None,
            glyph_count: 0,
        }
    }

    pub fn from_gpu_instances(device: &wgpu::Device, instances: &[GpuTextGlyphInstance]) -> Self {
        if instances.is_empty() {
            return Self::empty();
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("w text overlay glyph instances"),
            contents: cast_slice(instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            buffer: Some(buffer),
            glyph_count: instances.len() as u32,
        }
    }

    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[GpuTextGlyphInstance],
    ) {
        if instances.is_empty() {
            self.buffer = None;
            self.glyph_count = 0;
            return;
        }

        let contents = cast_slice(instances);
        let needs_new_buffer = self
            .buffer
            .as_ref()
            .map(|buffer| buffer.size() < contents.len() as u64)
            .unwrap_or(true);
        if needs_new_buffer {
            *self = Self::from_gpu_instances(device, instances);
        } else if let Some(buffer) = &self.buffer {
            queue.write_buffer(buffer, 0, contents);
            self.glyph_count = instances.len() as u32;
        }
    }

    pub fn glyph_count(&self) -> u32 {
        self.glyph_count
    }

    pub fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }
}

pub struct TextOverlayRenderer {
    overlay_pipeline: TextOverlayPipeline,
    depth_tested_pipeline: TextOverlayPipeline,
    xray_pipeline: TextOverlayPipeline,
}

impl TextOverlayRenderer {
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let atlas_bind_group_layout = create_text_atlas_bind_group_layout(device);
        Self {
            overlay_pipeline: TextOverlayPipeline::with_atlas_bind_group_layout(
                device,
                color_format,
                Some(depth_format),
                camera_bind_group_layout,
                &atlas_bind_group_layout,
                SceneTextDepthMode::Overlay,
            ),
            depth_tested_pipeline: TextOverlayPipeline::with_atlas_bind_group_layout(
                device,
                color_format,
                Some(depth_format),
                camera_bind_group_layout,
                &atlas_bind_group_layout,
                SceneTextDepthMode::DepthTested,
            ),
            xray_pipeline: TextOverlayPipeline::with_atlas_bind_group_layout(
                device,
                color_format,
                Some(depth_format),
                camera_bind_group_layout,
                &atlas_bind_group_layout,
                SceneTextDepthMode::XRay,
            ),
        }
    }

    pub fn atlas_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        self.overlay_pipeline.atlas_bind_group_layout()
    }

    pub fn render_overlay(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
        camera_bind_group: &wgpu::BindGroup,
        atlas: &TextAtlas,
        glyphs: &TextOverlayGlyphBuffer,
    ) {
        self.render_with_pipeline(
            &self.overlay_pipeline,
            encoder,
            target,
            depth_target,
            camera_bind_group,
            atlas.bind_group(),
            glyphs,
            "w text overlay pass",
        );
    }

    /// DepthTested and XRay use the same glyph data path as Overlay. The only
    /// policy difference is depth comparison: future integration can bucket
    /// `SceneTextLabel::depth_mode` and call the matching render method.
    pub fn render_depth_tested(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
        camera_bind_group: &wgpu::BindGroup,
        atlas: &TextAtlas,
        glyphs: &TextOverlayGlyphBuffer,
    ) {
        self.render_with_pipeline(
            &self.depth_tested_pipeline,
            encoder,
            target,
            depth_target,
            camera_bind_group,
            atlas.bind_group(),
            glyphs,
            "w text depth-tested pass",
        );
    }

    pub fn render_xray(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
        camera_bind_group: &wgpu::BindGroup,
        atlas: &TextAtlas,
        glyphs: &TextOverlayGlyphBuffer,
    ) {
        self.render_with_pipeline(
            &self.xray_pipeline,
            encoder,
            target,
            depth_target,
            camera_bind_group,
            atlas.bind_group(),
            glyphs,
            "w text xray pass",
        );
    }

    fn render_with_pipeline(
        &self,
        pipeline: &TextOverlayPipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth_target: &wgpu::TextureView,
        camera_bind_group: &wgpu::BindGroup,
        atlas_bind_group: &wgpu::BindGroup,
        glyphs: &TextOverlayGlyphBuffer,
        label: &'static str,
    ) {
        let Some(glyph_buffer) = glyphs.buffer() else {
            return;
        };
        if glyphs.glyph_count() == 0 {
            return;
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
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
        pass.set_pipeline(pipeline.pipeline());
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, atlas_bind_group, &[]);
        pass.set_vertex_buffer(0, glyph_buffer.slice(..));
        pass.draw(0..TEXT_GLYPH_VERTICES_PER_INSTANCE, 0..glyphs.glyph_count());
    }
}

pub fn create_text_camera_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("w text overlay camera bind group layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

pub fn create_text_camera_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    camera_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("w text overlay camera bind group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_buffer.as_entire_binding(),
        }],
    })
}

pub fn align_label_origin_px(
    horizontal: SceneTextHorizontalAlign,
    vertical: SceneTextVerticalAlign,
    label_size_px: DVec2,
    baseline_px: f64,
) -> DVec2 {
    let x = match horizontal {
        SceneTextHorizontalAlign::Left => 0.0,
        SceneTextHorizontalAlign::Center => -label_size_px.x * 0.5,
        SceneTextHorizontalAlign::Right => -label_size_px.x,
    };
    let y = match vertical {
        SceneTextVerticalAlign::Top => 0.0,
        SceneTextVerticalAlign::Middle => -label_size_px.y * 0.5,
        SceneTextVerticalAlign::Bottom => -label_size_px.y,
        SceneTextVerticalAlign::Baseline => -baseline_px,
    };
    DVec2::new(x, y)
}

pub fn label_base_offset_px(
    label: &SceneTextLabel,
    label_size_px: DVec2,
    baseline_px: f64,
) -> DVec2 {
    align_label_origin_px(
        label.horizontal_align,
        label.vertical_align,
        label_size_px,
        baseline_px,
    ) + label.screen_offset_px
}

pub fn project_label_anchor_px(
    clip_from_world: DMat4,
    viewport_width: f64,
    viewport_height: f64,
    anchor: DVec3,
) -> Option<DVec2> {
    if viewport_width <= 0.0 || viewport_height <= 0.0 || !anchor.is_finite() {
        return None;
    }
    let clip = clip_from_world * DVec4::new(anchor.x, anchor.y, anchor.z, 1.0);
    if !clip.is_finite() || clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some(DVec2::new(
        (ndc.x * 0.5 + 0.5) * viewport_width,
        (0.5 - ndc.y * 0.5) * viewport_height,
    ))
}

pub fn text_glyphs_from_layout(
    label: &SceneTextLabel,
    layout: &cc_w_text::TextLayout,
    sdf_pixel_range: f32,
    outline_width_px: f32,
) -> Vec<TextGlyph> {
    let [red, green, blue] = label.style.color.as_rgb();
    let color = [red, green, blue, label.style.color_alpha];
    let outline_color = label
        .style
        .outline_color
        .map(|outline| {
            let [red, green, blue] = outline.as_rgb();
            [red, green, blue, label.style.outline_alpha]
        })
        .unwrap_or([0.0, 0.0, 0.0, 0.0]);

    layout
        .glyphs
        .iter()
        .filter_map(|glyph| {
            let width = glyph.quad.width();
            let height = glyph.quad.height();
            if width <= 0.0 || height <= 0.0 {
                return None;
            }
            let uv_width = glyph.uv_rect.max_u - glyph.uv_rect.min_u;
            let uv_height = glyph.uv_rect.max_v - glyph.uv_rect.min_v;
            Some(TextGlyph {
                anchor_world: label.anchor,
                rect_px: [glyph.quad.min_x, glyph.quad.min_y, width, height],
                uv_rect: [
                    glyph.uv_rect.min_u,
                    glyph.uv_rect.min_v,
                    uv_width,
                    uv_height,
                ],
                color,
                outline_color,
                sdf_pixel_range,
                outline_width_px,
                embolden_px: label.style.embolden_px,
                depth_mode: label.depth_mode,
            })
        })
        .collect()
}

pub fn depth_compare_for_text_mode(mode: SceneTextDepthMode) -> wgpu::CompareFunction {
    match mode {
        SceneTextDepthMode::Overlay | SceneTextDepthMode::XRay => wgpu::CompareFunction::Always,
        SceneTextDepthMode::DepthTested => wgpu::CompareFunction::LessEqual,
    }
}

pub fn depth_mode_code(mode: SceneTextDepthMode) -> f32 {
    match mode {
        SceneTextDepthMode::Overlay => 0.0,
        SceneTextDepthMode::DepthTested => 1.0,
        SceneTextDepthMode::XRay => 2.0,
    }
}

pub fn bytes_per_texel(format: wgpu::TextureFormat) -> u32 {
    match format {
        wgpu::TextureFormat::R8Unorm => 1,
        wgpu::TextureFormat::Rg8Unorm => 2,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => 4,
        _ => 0,
    }
}

pub fn create_text_atlas_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("w text overlay atlas bind group layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

pub fn create_text_atlas_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("w text overlay atlas bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn sanitize_color(mut color: [f32; 4]) -> [f32; 4] {
    for component in &mut color {
        if !component.is_finite() {
            *component = 0.0;
        }
    }
    color[3] = color[3].clamp(0.0, 1.0);
    color
}

fn all_finite(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite())
}

#[allow(dead_code)]
fn assert_camera_uniform_compatible<T: Pod + Zeroable>(camera_uniform: &T) {
    let _ = bytes_of(camera_uniform);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    fn workspace_relative_path(path: impl AsRef<std::path::Path>) -> std::path::PathBuf {
        let path = path.as_ref();
        if path.is_absolute() {
            return path.to_path_buf();
        }
        let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = crate_dir
            .parent()
            .and_then(std::path::Path::parent)
            .expect("cc-w-render crate lives under workspace crates/");
        workspace_root.join(path)
    }

    #[test]
    fn label_alignment_offsets_match_contracts() {
        let size = DVec2::new(120.0, 40.0);
        assert_eq!(
            align_label_origin_px(
                SceneTextHorizontalAlign::Center,
                SceneTextVerticalAlign::Middle,
                size,
                28.0
            ),
            DVec2::new(-60.0, -20.0)
        );
        assert_eq!(
            align_label_origin_px(
                SceneTextHorizontalAlign::Right,
                SceneTextVerticalAlign::Baseline,
                size,
                28.0
            ),
            DVec2::new(-120.0, -28.0)
        );
    }

    #[test]
    fn glyph_instance_layout_stays_wgsl_aligned() {
        assert_eq!(std::mem::size_of::<GpuTextGlyphInstance>(), 96);
        assert_eq!(std::mem::align_of::<GpuTextGlyphInstance>(), 4);
        assert_eq!(GpuTextGlyphInstance::layout().array_stride, 96);
    }

    #[test]
    fn shader_keeps_renderer_camera_layout_names() {
        for expected in [
            "clip_from_world",
            "viewport_and_profile",
            "view_from_world",
            "clip_plane",
            "clip_params",
        ] {
            assert!(TEXT_OVERLAY_SHADER_WGSL.contains(expected));
        }
    }

    #[test]
    fn shader_interprets_outline_width_as_pixels() {
        assert!(
            TEXT_OVERLAY_SHADER_WGSL
                .contains("let outline_radius = min(max(input.sdf_params.y, 0.0), 4.0)"),
            "outline width is a hard bitmap dilation radius in screen pixels"
        );
    }

    #[test]
    fn shader_interprets_embolden_as_pixels() {
        assert!(
            TEXT_OVERLAY_SHADER_WGSL.contains("0.5 - max(input.sdf_params.w, 0.0) * 0.25"),
            "embolden amount should lower the bitmap mask threshold"
        );
    }

    #[test]
    fn shader_uses_binary_bitmap_thresholds_for_hard_edges() {
        assert!(
            TEXT_OVERLAY_SHADER_WGSL.contains("select(0.0, 1.0, center_alpha >= fill_threshold)")
        );
        assert!(TEXT_OVERLAY_SHADER_WGSL.contains("textureLoad(text_atlas"));
    }

    #[test]
    fn text_atlas_uses_nearest_filtering() {
        assert_eq!(TEXT_ATLAS_FILTER_MODE, wgpu::FilterMode::Nearest);
    }

    #[test]
    fn depth_modes_map_to_pipeline_compare_functions() {
        assert_eq!(
            depth_compare_for_text_mode(SceneTextDepthMode::Overlay),
            wgpu::CompareFunction::Always
        );
        assert_eq!(
            depth_compare_for_text_mode(SceneTextDepthMode::DepthTested),
            wgpu::CompareFunction::LessEqual
        );
        assert_eq!(
            depth_compare_for_text_mode(SceneTextDepthMode::XRay),
            wgpu::CompareFunction::Always
        );
    }

    #[test]
    fn rejects_non_finite_glyphs_before_upload() {
        let glyph = TextGlyph {
            anchor_world: DVec3::new(f64::NAN, 0.0, 0.0),
            rect_px: [0.0, 0.0, 10.0, 10.0],
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            color: [1.0; 4],
            outline_color: [0.0; 4],
            sdf_pixel_range: DEFAULT_SDF_PIXEL_RANGE,
            outline_width_px: 0.0,
            embolden_px: 0.0,
            depth_mode: SceneTextDepthMode::Overlay,
        };
        assert!(glyph.to_gpu().is_none());
    }

    #[test]
    fn converts_cpu_text_layout_into_render_glyphs() {
        let mut label = SceneTextLabel::new("station-120", "120", DVec3::new(1.0, 2.0, 3.0));
        label.depth_mode = SceneTextDepthMode::DepthTested;
        label.style.color_alpha = 0.8;
        label.style.outline_color = Some(cc_w_types::DisplayColor::new(0.1, 0.2, 0.3));
        label.style.outline_alpha = 0.4;
        label.style.embolden_px = 0.6;
        let layout = cc_w_text::TextLayout {
            glyphs: vec![cc_w_text::LaidOutGlyph {
                key: cc_w_text::GlyphRasterKey {
                    glyph_id: 1,
                    size_px_64: 14 * 64,
                },
                glyph_id: 1,
                quad: cc_w_text::PixelRect {
                    min_x: -4.0,
                    min_y: -8.0,
                    max_x: 6.0,
                    max_y: 12.0,
                },
                uv_rect: cc_w_text::UvRect {
                    min_u: 0.25,
                    min_v: 0.5,
                    max_u: 0.5,
                    max_v: 0.75,
                },
            }],
            bounds: cc_w_text::PixelRect {
                min_x: -4.0,
                min_y: -8.0,
                max_x: 6.0,
                max_y: 12.0,
            },
            text_bounds: cc_w_text::PixelRect {
                min_x: -4.0,
                min_y: -8.0,
                max_x: 6.0,
                max_y: 12.0,
            },
        };

        let glyphs = text_glyphs_from_layout(&label, &layout, 4.0, 1.5);
        assert_eq!(glyphs.len(), 1);
        assert_eq!(glyphs[0].anchor_world, DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(glyphs[0].rect_px, [-4.0, -8.0, 10.0, 20.0]);
        assert_eq!(glyphs[0].uv_rect, [0.25, 0.5, 0.25, 0.25]);
        assert_eq!(glyphs[0].color[3], 0.8);
        assert_eq!(glyphs[0].outline_color, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(glyphs[0].embolden_px, 0.6);
        assert_eq!(glyphs[0].depth_mode, SceneTextDepthMode::DepthTested);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn renders_sdf_text_overlay_to_offscreen_target() {
        use bytemuck::{Pod, Zeroable, bytes_of};
        use std::sync::mpsc;

        #[repr(C)]
        #[derive(Clone, Copy, Pod, Zeroable)]
        struct TestCameraUniform {
            clip_from_world: [[f32; 4]; 4],
            viewport_and_profile: [f32; 4],
            view_from_world: [[f32; 4]; 4],
            clip_plane: [f32; 4],
            clip_params: [f32; 4],
        }

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
                    label: Some("w text overlay smoke test device"),
                    ..Default::default()
                })
                .await
                .expect("device");

            let width = 160;
            let height = 96;
            let color_format = wgpu::TextureFormat::Rgba8Unorm;
            let depth_format = wgpu::TextureFormat::Depth32Float;
            let color_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("w text overlay smoke color texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("w text overlay smoke depth texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: depth_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

            let camera_bind_group_layout = create_text_camera_bind_group_layout(&device);
            let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("w text overlay smoke camera buffer"),
                contents: bytes_of(&TestCameraUniform {
                    clip_from_world: DMat4::IDENTITY
                        .to_cols_array_2d()
                        .map(|col| [col[0] as f32, col[1] as f32, col[2] as f32, col[3] as f32]),
                    viewport_and_profile: [width as f32, height as f32, 0.0, 0.0],
                    view_from_world: DMat4::IDENTITY
                        .to_cols_array_2d()
                        .map(|col| [col[0] as f32, col[1] as f32, col[2] as f32, col[3] as f32]),
                    clip_plane: [0.0, 0.0, 0.0, 0.0],
                    clip_params: [0.0, 0.0, 0.0, 0.0],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let camera_bind_group =
                create_text_camera_bind_group(&device, &camera_bind_group_layout, &camera_buffer);
            let renderer = TextOverlayRenderer::new(
                &device,
                color_format,
                depth_format,
                &camera_bind_group_layout,
            );

            let font = cc_w_text::TextFont::from_bytes(epaint_default_fonts::HACK_REGULAR.to_vec())
                .expect("default test font");
            let mut cpu_atlas = cc_w_text::TextAtlas::new_alpha_mask(256, 128, 4);
            let mut label = SceneTextLabel::new("station-120", "120", DVec3::ZERO);
            label.style.size_px = 32.0;
            label.style.color = cc_w_types::DisplayColor::new(1.0, 1.0, 1.0);
            label.style.outline_color = Some(cc_w_types::DisplayColor::new(0.0, 0.0, 0.0));
            label.style.embolden_px = 0.5;
            let layout = cc_w_text::layout_label(&font, &mut cpu_atlas, &label).expect("layout");
            let glyphs =
                text_glyphs_from_layout(&label, &layout, cpu_atlas.sdf_radius_px() as f32, 1.25);
            let glyph_corner_samples = glyphs
                .iter()
                .flat_map(|glyph| {
                    let x0 = width as f32 * 0.5 + glyph.rect_px[0] + 1.0;
                    let y0 = height as f32 * 0.5 + glyph.rect_px[1] + 1.0;
                    let x1 = width as f32 * 0.5 + glyph.rect_px[0] + glyph.rect_px[2] - 2.0;
                    let y1 = height as f32 * 0.5 + glyph.rect_px[1] + glyph.rect_px[3] - 2.0;
                    [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
                })
                .filter_map(|(x, y)| {
                    if x >= 0.0 && y >= 0.0 && x < width as f32 && y < height as f32 {
                        Some((x.round() as u32, y.round() as u32))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            assert!(!glyph_corner_samples.is_empty());
            let gpu_instances = glyphs
                .into_iter()
                .filter_map(TextGlyph::to_gpu)
                .collect::<Vec<_>>();
            assert!(!gpu_instances.is_empty());
            let glyph_buffer = TextOverlayGlyphBuffer::from_gpu_instances(&device, &gpu_instances);
            let gpu_atlas = TextAtlas::new(
                &device,
                &queue,
                renderer.atlas_bind_group_layout(),
                TextAtlasDescriptor::sdf_r8(cpu_atlas.width(), cpu_atlas.height()),
                Some(cpu_atlas.pixels()),
            );

            let unpadded_bytes_per_row = width * 4;
            let padded_bytes_per_row =
                crate::align_to(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let readback_size = u64::from(padded_bytes_per_row) * u64::from(height);
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("w text overlay smoke readback buffer"),
                size: readback_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("w text overlay smoke encoder"),
            });
            {
                let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("w text overlay smoke clear pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.33,
                                g: 0.37,
                                b: 0.41,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            renderer.render_overlay(
                &mut encoder,
                &color_view,
                &depth_view,
                &camera_bind_group,
                &gpu_atlas,
                &glyph_buffer,
            );
            encoder.copy_texture_to_buffer(
                color_texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            let submission = queue.submit([encoder.finish()]);
            let slice = readback.slice(..);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: None,
                })
                .expect("device poll");
            rx.recv().expect("readback callback").expect("readback map");

            let mapped = slice.get_mapped_range();
            let rgba8 = crate::strip_padded_rows(
                &mapped,
                unpadded_bytes_per_row as usize,
                padded_bytes_per_row as usize,
                height as usize,
            );
            drop(mapped);
            readback.unmap();

            let bright_pixels = rgba8
                .chunks_exact(4)
                .filter(|pixel| {
                    u16::from(pixel[0]) + u16::from(pixel[1]) + u16::from(pixel[2]) > 600
                })
                .count();
            assert!(
                bright_pixels > 40,
                "expected visible text pixels, found {bright_pixels}"
            );
            let dark_corner_samples = glyph_corner_samples
                .iter()
                .filter(|(x, y)| {
                    let offset = ((*y as usize * width as usize) + *x as usize) * 4;
                    let pixel = &rgba8[offset..offset + 4];
                    u16::from(pixel[0]) + u16::from(pixel[1]) + u16::from(pixel[2]) < 96
                })
                .count();
            assert_eq!(
                dark_corner_samples, 0,
                "outlined text should not fill transparent glyph-quad corners"
            );
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "visual fixture; writes CC_W_TEXT_OVERLAY_VISUAL_OUT when set"]
    fn dump_text_overlay_visual_fixture() {
        use bytemuck::{Pod, Zeroable, bytes_of};
        use std::sync::mpsc;

        #[repr(C)]
        #[derive(Clone, Copy, Pod, Zeroable)]
        struct TestCameraUniform {
            clip_from_world: [[f32; 4]; 4],
            viewport_and_profile: [f32; 4],
            view_from_world: [[f32; 4]; 4],
            clip_plane: [f32; 4],
            clip_params: [f32; 4],
        }

        fn label(
            id: &str,
            text: &str,
            anchor: DVec3,
            size_px: f32,
            color: [f32; 3],
            outline: [f32; 3],
        ) -> SceneTextLabel {
            let mut label = SceneTextLabel::new(id, text, anchor);
            label.horizontal_align = SceneTextHorizontalAlign::Left;
            label.vertical_align = SceneTextVerticalAlign::Middle;
            label.style.size_px = size_px;
            label.style.color = cc_w_types::DisplayColor::new(color[0], color[1], color[2]);
            label.style.outline_color = Some(cc_w_types::DisplayColor::new(
                outline[0], outline[1], outline[2],
            ));
            label
        }

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
                    label: Some("w text overlay visual fixture device"),
                    ..Default::default()
                })
                .await
                .expect("device");

            let width = 1200;
            let height = 760;
            let color_format = wgpu::TextureFormat::Rgba8Unorm;
            let depth_format = wgpu::TextureFormat::Depth32Float;
            let color_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("w text overlay visual fixture color texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("w text overlay visual fixture depth texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: depth_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

            let camera_bind_group_layout = create_text_camera_bind_group_layout(&device);
            let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("w text overlay visual fixture camera buffer"),
                contents: bytes_of(&TestCameraUniform {
                    clip_from_world: DMat4::IDENTITY
                        .to_cols_array_2d()
                        .map(|col| [col[0] as f32, col[1] as f32, col[2] as f32, col[3] as f32]),
                    viewport_and_profile: [width as f32, height as f32, 0.0, 0.0],
                    view_from_world: DMat4::IDENTITY
                        .to_cols_array_2d()
                        .map(|col| [col[0] as f32, col[1] as f32, col[2] as f32, col[3] as f32]),
                    clip_plane: [0.0, 0.0, 0.0, 0.0],
                    clip_params: [0.0, 0.0, 0.0, 0.0],
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let camera_bind_group =
                create_text_camera_bind_group(&device, &camera_bind_group_layout, &camera_buffer);
            let renderer = TextOverlayRenderer::new(
                &device,
                color_format,
                depth_format,
                &camera_bind_group_layout,
            );

            let font = cc_w_text::TextFont::from_bytes(epaint_default_fonts::HACK_REGULAR.to_vec())
                .expect("default test font");
            let mut cpu_atlas = cc_w_text::TextAtlas::new_alpha_mask(2048, 1024, 6);
            let mut labels = vec![
                label(
                    "fixture-title",
                    "W text overlay glyph fixture",
                    DVec3::new(-0.86, 0.78, 0.0),
                    44.0,
                    [0.92, 0.97, 1.0],
                    [0.02, 0.04, 0.06],
                ),
                label(
                    "fixture-caption",
                    "SDF atlas, one GPU overlay pass, identity clip-space anchors",
                    DVec3::new(-0.86, 0.63, 0.0),
                    18.0,
                    [0.62, 0.72, 0.84],
                    [0.02, 0.04, 0.06],
                ),
                label(
                    "station-12",
                    "station 120",
                    DVec3::new(-0.86, 0.42, 0.0),
                    12.0,
                    [0.92, 0.96, 1.0],
                    [0.02, 0.04, 0.06],
                ),
                label(
                    "station-18",
                    "station 120",
                    DVec3::new(-0.86, 0.28, 0.0),
                    18.0,
                    [0.92, 0.96, 1.0],
                    [0.02, 0.04, 0.06],
                ),
                label(
                    "station-24",
                    "station 120",
                    DVec3::new(-0.86, 0.10, 0.0),
                    24.0,
                    [0.93, 0.97, 1.0],
                    [0.02, 0.04, 0.06],
                ),
                label(
                    "bridge-alignment",
                    "Bridge alignment A",
                    DVec3::new(-0.86, -0.13, 0.0),
                    32.0,
                    [0.78, 0.90, 1.0],
                    [0.01, 0.03, 0.05],
                ),
                label(
                    "chainage",
                    "CHAINAGE 120.000 m",
                    DVec3::new(-0.86, -0.43, 0.0),
                    48.0,
                    [1.0, 0.80, 0.44],
                    [0.03, 0.02, 0.01],
                ),
                label(
                    "delta",
                    "Delta x 4.2 m   slope 2.5%",
                    DVec3::new(-0.86, -0.74, 0.0),
                    64.0,
                    [0.66, 1.0, 0.83],
                    [0.00, 0.05, 0.03],
                ),
                label(
                    "right-small",
                    "right aligned / baseline",
                    DVec3::new(0.84, 0.42, 0.0),
                    22.0,
                    [0.78, 0.84, 0.95],
                    [0.01, 0.03, 0.05],
                ),
                label(
                    "right-large",
                    "156.75",
                    DVec3::new(0.84, 0.20, 0.0),
                    72.0,
                    [0.94, 0.96, 1.0],
                    [0.01, 0.03, 0.05],
                ),
            ];
            labels[8].horizontal_align = SceneTextHorizontalAlign::Right;
            labels[8].vertical_align = SceneTextVerticalAlign::Baseline;
            labels[9].horizontal_align = SceneTextHorizontalAlign::Right;
            labels[9].vertical_align = SceneTextVerticalAlign::Baseline;

            let mut glyphs = Vec::new();
            for label in &labels {
                let layout =
                    cc_w_text::layout_label(&font, &mut cpu_atlas, label).expect("layout label");
                glyphs.extend(text_glyphs_from_layout(
                    label,
                    &layout,
                    cpu_atlas.sdf_radius_px() as f32,
                    1.4,
                ));
            }
            let gpu_instances = glyphs
                .into_iter()
                .filter_map(TextGlyph::to_gpu)
                .collect::<Vec<_>>();
            assert!(!gpu_instances.is_empty());
            let glyph_buffer = TextOverlayGlyphBuffer::from_gpu_instances(&device, &gpu_instances);
            let gpu_atlas = TextAtlas::new(
                &device,
                &queue,
                renderer.atlas_bind_group_layout(),
                TextAtlasDescriptor::sdf_r8(cpu_atlas.width(), cpu_atlas.height()),
                Some(cpu_atlas.pixels()),
            );

            let unpadded_bytes_per_row = width * 4;
            let padded_bytes_per_row =
                crate::align_to(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let readback_size = u64::from(padded_bytes_per_row) * u64::from(height);
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("w text overlay visual fixture readback buffer"),
                size: readback_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("w text overlay visual fixture encoder"),
            });
            {
                let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("w text overlay visual fixture clear pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.055,
                                g: 0.064,
                                b: 0.078,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            renderer.render_overlay(
                &mut encoder,
                &color_view,
                &depth_view,
                &camera_bind_group,
                &gpu_atlas,
                &glyph_buffer,
            );
            encoder.copy_texture_to_buffer(
                color_texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            let submission = queue.submit([encoder.finish()]);
            let slice = readback.slice(..);
            let (tx, rx) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: None,
                })
                .expect("device poll");
            rx.recv().expect("readback callback").expect("readback map");

            let mapped = slice.get_mapped_range();
            let rgba8 = crate::strip_padded_rows(
                &mapped,
                unpadded_bytes_per_row as usize,
                padded_bytes_per_row as usize,
                height as usize,
            );
            drop(mapped);
            readback.unmap();

            let bright_pixels = rgba8
                .chunks_exact(4)
                .filter(|pixel| {
                    u16::from(pixel[0]) + u16::from(pixel[1]) + u16::from(pixel[2]) > 180
                })
                .count();
            assert!(
                bright_pixels > 2_000,
                "expected visible fixture text pixels, found {bright_pixels}"
            );

            if let Some(path) = std::env::var_os("CC_W_TEXT_OVERLAY_VISUAL_OUT") {
                let path = workspace_relative_path(std::path::PathBuf::from(path));
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("create visual fixture directory");
                }
                let file = std::fs::File::create(&path).expect("create visual fixture png");
                let mut encoder = png::Encoder::new(file, width, height);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().expect("png header");
                writer.write_image_data(&rgba8).expect("png data");
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "visual fixture; writes a PNG preview from the CPU SDF atlas"]
    fn dump_text_glyph_visual_fixture() {
        fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
            let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }

        fn blend_over(dst: &mut [u8], src: [f32; 4]) {
            let alpha = src[3].clamp(0.0, 1.0);
            let inv = 1.0 - alpha;
            for channel in 0..3 {
                let dst_value = f32::from(dst[channel]) / 255.0;
                dst[channel] = ((src[channel].clamp(0.0, 1.0) * alpha + dst_value * inv) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            dst[3] = 255;
        }

        fn sample_atlas_bilinear(
            pixels: &[u8],
            width: usize,
            height: usize,
            u: f32,
            v: f32,
        ) -> f32 {
            let x = (u * width as f32 - 0.5).clamp(0.0, (width - 1) as f32);
            let y = (v * height as f32 - 0.5).clamp(0.0, (height - 1) as f32);
            let x0 = x.floor() as usize;
            let y0 = y.floor() as usize;
            let x1 = (x0 + 1).min(width - 1);
            let y1 = (y0 + 1).min(height - 1);
            let tx = x - x0 as f32;
            let ty = y - y0 as f32;
            let p00 = f32::from(pixels[y0 * width + x0]) / 255.0;
            let p10 = f32::from(pixels[y0 * width + x1]) / 255.0;
            let p01 = f32::from(pixels[y1 * width + x0]) / 255.0;
            let p11 = f32::from(pixels[y1 * width + x1]) / 255.0;
            let top = p00 + (p10 - p00) * tx;
            let bottom = p01 + (p11 - p01) * tx;
            top + (bottom - top) * ty
        }

        fn label(
            id: &str,
            text: &str,
            anchor: DVec3,
            size_px: f32,
            color: [f32; 3],
            outline: [f32; 3],
        ) -> SceneTextLabel {
            let mut label = SceneTextLabel::new(id, text, anchor);
            label.horizontal_align = SceneTextHorizontalAlign::Left;
            label.vertical_align = SceneTextVerticalAlign::Middle;
            label.style.size_px = size_px;
            label.style.color = cc_w_types::DisplayColor::new(color[0], color[1], color[2]);
            label.style.outline_color = Some(cc_w_types::DisplayColor::new(
                outline[0], outline[1], outline[2],
            ));
            label
        }

        let width = 1200u32;
        let height = 760u32;
        let font = cc_w_text::TextFont::from_bytes(epaint_default_fonts::HACK_REGULAR.to_vec())
            .expect("default test font");
        let mut cpu_atlas = cc_w_text::TextAtlas::new_alpha_mask(2048, 1024, 6);
        let mut labels = vec![
            label(
                "fixture-title",
                "W text overlay glyph fixture",
                DVec3::new(-0.86, 0.78, 0.0),
                44.0,
                [0.92, 0.97, 1.0],
                [0.02, 0.04, 0.06],
            ),
            label(
                "fixture-caption",
                "CPU preview from the same font layout and SDF atlas used by the renderer",
                DVec3::new(-0.86, 0.63, 0.0),
                18.0,
                [0.62, 0.72, 0.84],
                [0.02, 0.04, 0.06],
            ),
            label(
                "station-12",
                "station 120",
                DVec3::new(-0.86, 0.42, 0.0),
                12.0,
                [0.92, 0.96, 1.0],
                [0.02, 0.04, 0.06],
            ),
            label(
                "station-18",
                "station 120",
                DVec3::new(-0.86, 0.28, 0.0),
                18.0,
                [0.92, 0.96, 1.0],
                [0.02, 0.04, 0.06],
            ),
            label(
                "station-24",
                "station 120",
                DVec3::new(-0.86, 0.10, 0.0),
                24.0,
                [0.93, 0.97, 1.0],
                [0.02, 0.04, 0.06],
            ),
            label(
                "bridge-alignment",
                "Bridge alignment A",
                DVec3::new(-0.86, -0.13, 0.0),
                32.0,
                [0.78, 0.90, 1.0],
                [0.01, 0.03, 0.05],
            ),
            label(
                "chainage",
                "CHAINAGE 120.000 m",
                DVec3::new(-0.86, -0.43, 0.0),
                48.0,
                [1.0, 0.80, 0.44],
                [0.03, 0.02, 0.01],
            ),
            label(
                "delta",
                "Delta x 4.2 m   slope 2.5%",
                DVec3::new(-0.86, -0.74, 0.0),
                64.0,
                [0.66, 1.0, 0.83],
                [0.00, 0.05, 0.03],
            ),
            label(
                "right-small",
                "right aligned / baseline",
                DVec3::new(0.84, 0.42, 0.0),
                22.0,
                [0.78, 0.84, 0.95],
                [0.01, 0.03, 0.05],
            ),
            label(
                "right-large",
                "156.75",
                DVec3::new(0.84, 0.20, 0.0),
                72.0,
                [0.94, 0.96, 1.0],
                [0.01, 0.03, 0.05],
            ),
        ];
        labels[8].horizontal_align = SceneTextHorizontalAlign::Right;
        labels[8].vertical_align = SceneTextVerticalAlign::Baseline;
        labels[9].horizontal_align = SceneTextHorizontalAlign::Right;
        labels[9].vertical_align = SceneTextVerticalAlign::Baseline;

        let mut glyphs = Vec::new();
        for label in &labels {
            let layout =
                cc_w_text::layout_label(&font, &mut cpu_atlas, label).expect("layout label");
            glyphs.extend(text_glyphs_from_layout(
                label,
                &layout,
                cpu_atlas.sdf_radius_px() as f32,
                1.4,
            ));
        }

        let mut rgba8 = vec![0u8; width as usize * height as usize * 4];
        for pixel in rgba8.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[14, 16, 20, 255]);
        }

        let atlas_width = cpu_atlas.width() as usize;
        let atlas_height = cpu_atlas.height() as usize;
        let atlas_pixels = cpu_atlas.pixels();

        for glyph in glyphs {
            let anchor_px = DVec2::new(
                (glyph.anchor_world.x * 0.5 + 0.5) * f64::from(width),
                (-glyph.anchor_world.y * 0.5 + 0.5) * f64::from(height),
            );
            let left = anchor_px.x as f32 + glyph.rect_px[0];
            let top = anchor_px.y as f32 + glyph.rect_px[1];
            let glyph_width = glyph.rect_px[2].max(1.0);
            let glyph_height = glyph.rect_px[3].max(1.0);
            let x0 = left.floor().max(0.0) as u32;
            let y0 = top.floor().max(0.0) as u32;
            let x1 = (left + glyph_width).ceil().min(width as f32) as u32;
            let y1 = (top + glyph_height).ceil().min(height as f32) as u32;

            for y in y0..y1 {
                for x in x0..x1 {
                    let local_x = ((x as f32 + 0.5 - left) / glyph_width).clamp(0.0, 1.0);
                    let local_y = ((y as f32 + 0.5 - top) / glyph_height).clamp(0.0, 1.0);
                    let u = glyph.uv_rect[0] + local_x * glyph.uv_rect[2];
                    let v = glyph.uv_rect[1] + local_y * glyph.uv_rect[3];
                    let sdf = sample_atlas_bilinear(atlas_pixels, atlas_width, atlas_height, u, v);
                    let one_px_u = glyph.uv_rect[2] / glyph_width;
                    let one_px_v = glyph.uv_rect[3] / glyph_height;
                    let sdf_dx = sample_atlas_bilinear(
                        atlas_pixels,
                        atlas_width,
                        atlas_height,
                        u + one_px_u,
                        v,
                    );
                    let sdf_dy = sample_atlas_bilinear(
                        atlas_pixels,
                        atlas_width,
                        atlas_height,
                        u,
                        v + one_px_v,
                    );
                    let edge_width =
                        (((sdf_dx - sdf).abs() + (sdf_dy - sdf).abs()) * 0.5).max(0.0001);
                    let fill_center =
                        0.5 - glyph.embolden_px.max(0.0) / (2.0 * glyph.sdf_pixel_range);
                    let outline_center = fill_center
                        - glyph.outline_width_px.max(0.0) / (2.0 * glyph.sdf_pixel_range);
                    let fill_alpha =
                        smoothstep(fill_center - edge_width, fill_center + edge_width, sdf);
                    let outline_alpha = smoothstep(
                        outline_center - edge_width,
                        outline_center + edge_width,
                        sdf,
                    );
                    let outline_only = (outline_alpha - fill_alpha).max(0.0);
                    let alpha = glyph.color[3] * fill_alpha + glyph.outline_color[3] * outline_only;
                    if alpha <= 0.001 {
                        continue;
                    }
                    let rgb = [
                        glyph.outline_color[0] * outline_only + glyph.color[0] * fill_alpha,
                        glyph.outline_color[1] * outline_only + glyph.color[1] * fill_alpha,
                        glyph.outline_color[2] * outline_only + glyph.color[2] * fill_alpha,
                    ];
                    let src = if alpha > 0.0 {
                        [rgb[0] / alpha, rgb[1] / alpha, rgb[2] / alpha, alpha]
                    } else {
                        [0.0, 0.0, 0.0, 0.0]
                    };
                    let offset = (y as usize * width as usize + x as usize) * 4;
                    blend_over(&mut rgba8[offset..offset + 4], src);
                }
            }
        }

        let path = std::env::var_os("CC_W_TEXT_OVERLAY_VISUAL_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from("crates/cc-w-render/visual/text-overlay/text-overlay.png")
            });
        let path = workspace_relative_path(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create visual fixture directory");
        }
        let file = std::fs::File::create(&path).expect("create visual fixture png");
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(&rgba8).expect("png data");

        let bright_pixels = rgba8
            .chunks_exact(4)
            .filter(|pixel| u16::from(pixel[0]) + u16::from(pixel[1]) + u16::from(pixel[2]) > 180)
            .count();
        assert!(
            bright_pixels > 2_000,
            "expected visible fixture text pixels, found {bright_pixels}"
        );
    }
}
