use bytemuck::{Pod, Zeroable, cast_slice};
use egui::{
    ClippedPrimitive, Rect, TextureId,
    epaint::{self, Primitive},
};
use std::{collections::HashMap, ops::Range};
use wgpu::util::DeviceExt;
use wgpu::vertex_attr_array;

const EGUI_SHADER_WGSL: &str = r#"
struct ScreenUniform {
    size_in_points : vec2<f32>,
    _padding : vec2<u32>,
};

@group(0) @binding(0)
var<uniform> screen : ScreenUniform;

@group(1) @binding(0)
var ui_texture : texture_2d<f32>;

@group(1) @binding(1)
var ui_sampler : sampler;

struct VertexInput {
    @location(0) position : vec2<f32>,
    @location(1) uv : vec2<f32>,
    @location(2) color : vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) uv : vec2<f32>,
    @location(1) color : vec4<f32>,
};

@vertex
fn vs_main(input : VertexInput) -> VertexOutput {
    var out : VertexOutput;
    let ndc = vec2<f32>(
        (input.position.x / screen.size_in_points.x) * 2.0 - 1.0,
        1.0 - (input.position.y / screen.size_in_points.y) * 2.0,
    );
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = input.uv;
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input : VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(ui_texture, ui_sampler, input.uv);
    return texel * input.color;
}
"#;

pub struct EguiPainter {
    pipeline: wgpu::RenderPipeline,
    screen_buffer: wgpu::Buffer,
    screen_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    samplers: HashMap<egui::TextureOptions, wgpu::Sampler>,
    textures: HashMap<TextureId, EguiTexture>,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    index_buffer: wgpu::Buffer,
    index_capacity: u64,
}

pub struct PreparedEguiFrame {
    draws: Vec<EguiDraw>,
    pixels_per_point: f32,
    screen_size_in_pixels: [u32; 2],
    free_textures: Vec<TextureId>,
}

impl PreparedEguiFrame {
    pub fn free_textures(&self) -> &[TextureId] {
        &self.free_textures
    }

    fn is_empty(&self) -> bool {
        self.draws.is_empty()
    }
}

struct EguiTexture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    options: egui::TextureOptions,
}

struct EguiDraw {
    clip_rect: Rect,
    texture_id: TextureId,
    indices: Range<u32>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ScreenUniform {
    size_in_points: [f32; 2],
    _padding: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
}

impl GpuVertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
            vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Unorm8x4];

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        }
    }
}

impl EguiPainter {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let screen_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("w egui screen buffer"),
            contents: cast_slice(&[ScreenUniform {
                size_in_points: [1.0, 1.0],
                _padding: [0, 0],
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let screen_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("w egui screen bind group layout"),
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
            });
        let screen_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("w egui screen bind group"),
            layout: &screen_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen_buffer.as_entire_binding(),
            }],
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("w egui texture bind group layout"),
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
            });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("w egui shader"),
            source: wgpu::ShaderSource::Wgsl(EGUI_SHADER_WGSL.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("w egui pipeline layout"),
            bind_group_layouts: &[
                Some(&screen_bind_group_layout),
                Some(&texture_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w egui pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[GpuVertex::layout()],
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
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            screen_buffer,
            screen_bind_group,
            texture_bind_group_layout,
            samplers: HashMap::new(),
            textures: HashMap::new(),
            vertex_buffer: create_buffer(
                device,
                4,
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "w egui vertex buffer",
            ),
            vertex_capacity: 4,
            index_buffer: create_buffer(
                device,
                4,
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                "w egui index buffer",
            ),
            index_capacity: 4,
        }
    }

    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures_delta: &egui::TexturesDelta,
        clipped_primitives: &[ClippedPrimitive],
        screen_size_in_pixels: [u32; 2],
        pixels_per_point: f32,
    ) -> PreparedEguiFrame {
        for (texture_id, image_delta) in &textures_delta.set {
            self.update_texture(device, queue, *texture_id, image_delta);
        }

        let screen_uniform = ScreenUniform {
            size_in_points: [
                screen_size_in_pixels[0] as f32 / pixels_per_point,
                screen_size_in_pixels[1] as f32 / pixels_per_point,
            ],
            _padding: [0, 0],
        };
        queue.write_buffer(&self.screen_buffer, 0, cast_slice(&[screen_uniform]));

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut draws = Vec::new();

        for clipped in clipped_primitives {
            match &clipped.primitive {
                Primitive::Mesh(mesh) => {
                    let base_vertex = vertices.len() as u32;
                    let index_start = indices.len() as u32;

                    vertices.extend(mesh.vertices.iter().map(|vertex| GpuVertex {
                        position: [vertex.pos.x, vertex.pos.y],
                        uv: [vertex.uv.x, vertex.uv.y],
                        color: vertex.color.to_array(),
                    }));
                    indices.extend(mesh.indices.iter().map(|index| index + base_vertex));

                    draws.push(EguiDraw {
                        clip_rect: clipped.clip_rect,
                        texture_id: mesh.texture_id,
                        indices: index_start..indices.len() as u32,
                    });
                }
                Primitive::Callback(_) => {}
            }
        }

        let vertex_bytes = cast_slice(vertices.as_slice());
        if !vertex_bytes.is_empty() {
            self.ensure_vertex_capacity(device, vertex_bytes.len() as u64);
            queue.write_buffer(&self.vertex_buffer, 0, vertex_bytes);
        }

        let index_bytes = cast_slice(indices.as_slice());
        if !index_bytes.is_empty() {
            self.ensure_index_capacity(device, index_bytes.len() as u64);
            queue.write_buffer(&self.index_buffer, 0, index_bytes);
        }

        PreparedEguiFrame {
            draws,
            pixels_per_point,
            screen_size_in_pixels,
            free_textures: textures_delta.free.clone(),
        }
    }

    pub fn render<'pass>(
        &self,
        render_pass: &mut wgpu::RenderPass<'pass>,
        frame: &PreparedEguiFrame,
    ) {
        if frame.is_empty() {
            return;
        }

        render_pass.set_viewport(
            0.0,
            0.0,
            frame.screen_size_in_pixels[0] as f32,
            frame.screen_size_in_pixels[1] as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.screen_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

        for draw in &frame.draws {
            let Some(scissor) = clip_rect_to_scissor(
                draw.clip_rect,
                frame.pixels_per_point,
                frame.screen_size_in_pixels,
            ) else {
                continue;
            };
            let Some(texture) = self.textures.get(&draw.texture_id) else {
                continue;
            };

            render_pass.set_bind_group(1, &texture.bind_group, &[]);
            render_pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
            render_pass.draw_indexed(draw.indices.clone(), 0, 0..1);
        }
    }

    pub fn free_textures(&mut self, texture_ids: &[TextureId]) {
        for texture_id in texture_ids {
            if let Some(texture) = self.textures.remove(texture_id) {
                texture.texture.destroy();
            }
        }
    }

    fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_id: TextureId,
        image_delta: &epaint::ImageDelta,
    ) {
        let width = image_delta.image.width() as u32;
        let height = image_delta.image.height() as u32;
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let data_bytes: &[u8] = match &image_delta.image {
            epaint::ImageData::Color(image) => cast_slice(image.pixels.as_slice()),
        };

        let (texture, bind_group) = if let Some(pos) = image_delta.pos {
            let mut existing = self
                .textures
                .remove(&texture_id)
                .expect("partial egui texture update requires existing texture");
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &existing.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: pos[0] as u32,
                        y: pos[1] as u32,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                data_bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                extent,
            );

            if existing.options != image_delta.options {
                existing.bind_group = create_texture_bind_group(
                    device,
                    &self.texture_bind_group_layout,
                    &mut self.samplers,
                    &existing.texture,
                    image_delta.options,
                    texture_id,
                );
                existing.options = image_delta.options;
            }
            self.textures.insert(texture_id, existing);
            return;
        } else {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("w egui texture {texture_id:?}")),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
            });
            let bind_group = create_texture_bind_group(
                device,
                &self.texture_bind_group_layout,
                &mut self.samplers,
                &texture,
                image_delta.options,
                texture_id,
            );
            (texture, bind_group)
        };

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            extent,
        );

        self.textures.insert(
            texture_id,
            EguiTexture {
                texture,
                bind_group,
                options: image_delta.options,
            },
        );
    }

    fn ensure_vertex_capacity(&mut self, device: &wgpu::Device, required: u64) {
        if required <= self.vertex_capacity {
            return;
        }

        self.vertex_capacity = self.vertex_capacity.max(4).saturating_mul(2).max(required);
        self.vertex_buffer = create_buffer(
            device,
            self.vertex_capacity,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "w egui vertex buffer",
        );
    }

    fn ensure_index_capacity(&mut self, device: &wgpu::Device, required: u64) {
        if required <= self.index_capacity {
            return;
        }

        self.index_capacity = self.index_capacity.max(4).saturating_mul(2).max(required);
        self.index_buffer = create_buffer(
            device,
            self.index_capacity,
            wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            "w egui index buffer",
        );
    }
}

fn create_texture_bind_group(
    device: &wgpu::Device,
    texture_bind_group_layout: &wgpu::BindGroupLayout,
    samplers: &mut HashMap<egui::TextureOptions, wgpu::Sampler>,
    texture: &wgpu::Texture,
    options: egui::TextureOptions,
    texture_id: TextureId,
) -> wgpu::BindGroup {
    let sampler = samplers
        .entry(options)
        .or_insert_with(|| create_sampler(device, options));
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("w egui texture bind group {texture_id:?}")),
        layout: texture_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn create_sampler(device: &wgpu::Device, options: egui::TextureOptions) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("w egui sampler"),
        mag_filter: map_filter_mode(options.magnification),
        min_filter: map_filter_mode(options.minification),
        mipmap_filter: map_mipmap_filter_mode(options.mipmap_mode.unwrap_or(options.minification)),
        address_mode_u: map_wrap_mode(options.wrap_mode),
        address_mode_v: map_wrap_mode(options.wrap_mode),
        address_mode_w: map_wrap_mode(options.wrap_mode),
        ..Default::default()
    })
}

fn map_filter_mode(filter: egui::TextureFilter) -> wgpu::FilterMode {
    match filter {
        egui::TextureFilter::Nearest => wgpu::FilterMode::Nearest,
        egui::TextureFilter::Linear => wgpu::FilterMode::Linear,
    }
}

fn map_mipmap_filter_mode(filter: egui::TextureFilter) -> wgpu::MipmapFilterMode {
    match filter {
        egui::TextureFilter::Nearest => wgpu::MipmapFilterMode::Nearest,
        egui::TextureFilter::Linear => wgpu::MipmapFilterMode::Linear,
    }
}

fn map_wrap_mode(wrap: egui::TextureWrapMode) -> wgpu::AddressMode {
    match wrap {
        egui::TextureWrapMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        egui::TextureWrapMode::Repeat => wgpu::AddressMode::Repeat,
        egui::TextureWrapMode::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
    }
}

fn create_buffer(
    device: &wgpu::Device,
    size: u64,
    usage: wgpu::BufferUsages,
    label: &str,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(4),
        usage,
        mapped_at_creation: false,
    })
}

struct ScissorRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

fn clip_rect_to_scissor(
    clip_rect: Rect,
    pixels_per_point: f32,
    screen_size_in_pixels: [u32; 2],
) -> Option<ScissorRect> {
    let min_x = (clip_rect.min.x * pixels_per_point)
        .floor()
        .clamp(0.0, screen_size_in_pixels[0] as f32) as u32;
    let min_y = (clip_rect.min.y * pixels_per_point)
        .floor()
        .clamp(0.0, screen_size_in_pixels[1] as f32) as u32;
    let max_x = (clip_rect.max.x * pixels_per_point)
        .ceil()
        .clamp(min_x as f32, screen_size_in_pixels[0] as f32) as u32;
    let max_y = (clip_rect.max.y * pixels_per_point)
        .ceil()
        .clamp(min_y as f32, screen_size_in_pixels[1] as f32) as u32;

    let width = max_x.saturating_sub(min_x);
    let height = max_y.saturating_sub(min_y);
    if width == 0 || height == 0 {
        None
    } else {
        Some(ScissorRect {
            x: min_x,
            y: min_y,
            width,
            height,
        })
    }
}
