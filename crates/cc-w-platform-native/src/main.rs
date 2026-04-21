mod egui_painter;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use cc_w_backend::{
    DEFAULT_DEMO_RESOURCE, GeometryBackend, GeometryBackendError, ResourceError,
    available_demo_resources,
};
use cc_w_render::{
    Camera, DepthTarget, MeshRenderer, NullRenderBackend, RenderDefaults, ViewportSize,
    fit_camera_to_render_scene,
};
use cc_w_runtime::{DemoAsset, Engine, GeometryPackageSource, GeometryPackageSourceError};
use cc_w_types::{PreparedGeometryPackage, WORLD_FORWARD, WORLD_RIGHT, WORLD_UP};
use cc_w_velr::{
    VelrIfcModel, available_ifc_body_resources, default_ifc_artifacts_root, ifc_body_resource_name,
    parse_ifc_body_resource,
};
use egui::{
    Align, Align2, Color32, ComboBox, FontFamily, FontId, Layout, Order, Pos2, RichText, Stroke,
    TopBottomPanel, Vec2,
};
use egui_winit::{EventResponse as EguiEventResponse, State as EguiState};
use glam::DVec3;
use thiserror::Error;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Theme, Window, WindowAttributes, WindowId},
};

use crate::egui_painter::EguiPainter;

fn available_local_resources() -> Vec<String> {
    available_local_resources_at(&default_ifc_artifacts_root())
}

fn available_local_resources_at(ifc_artifacts_root: &Path) -> Vec<String> {
    let mut resources = available_demo_resources()
        .into_iter()
        .map(|resource| resource.to_string())
        .collect::<Vec<_>>();
    if let Ok(mut ifc_resources) = available_ifc_body_resources(ifc_artifacts_root) {
        resources.append(&mut ifc_resources);
    }
    resources
}

fn main() {
    let launch_options = match NativeLaunchOptions::from_env_args() {
        Ok(options) => options,
        Err(LaunchControlFlow::ExitSuccess(message)) => {
            println!("{message}");
            return;
        }
        Err(LaunchControlFlow::ExitFailure(message)) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    let event_loop = EventLoop::new().expect("failed to create native event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new(launch_options);
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("w native app failed: {error}");
        std::process::exit(1);
    }
}

struct App {
    state: Option<AppState>,
    launch_options: NativeLaunchOptions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NativeLaunchOptions {
    resource: String,
    auto_exit_after_frames: Option<u32>,
}

impl App {
    fn new(launch_options: NativeLaunchOptions) -> Self {
        Self {
            state: None,
            launch_options,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }

        match pollster::block_on(AppState::new(event_loop, &self.launch_options.resource)) {
            Ok(state) => {
                println!("w native renderer");
                println!(
                    "viewport seed: {}x{}",
                    state.config.width, state.config.height
                );
                println!("resource: {}", state.current_resource);
                println!("{}", state.demo_asset.summary_line());
                self.state = Some(state);
            }
            Err(error) => {
                eprintln!("w native renderer failed to initialize: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if window_id != state.window.id() {
            return;
        }

        let egui_response = state.on_window_event(&event);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. }
                if event.physical_key == PhysicalKey::Code(KeyCode::Escape) =>
            {
                event_loop.exit();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !egui_response.consumed => state.begin_drag(),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => state.end_drag(),
            WindowEvent::CursorLeft { .. } => state.end_drag(),
            WindowEvent::CursorMoved { position, .. } if !egui_response.consumed => {
                state.drag_to(position.x as f32, position.y as f32)
            }
            WindowEvent::MouseWheel { delta, .. } if !egui_response.consumed => {
                state.zoom(mouse_wheel_delta_y(delta))
            }
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::RedrawRequested => match state.render() {
                Ok(RenderStatus::Ok) => {
                    if state.advance_frame(self.launch_options.auto_exit_after_frames) {
                        event_loop.exit();
                    }
                }
                Ok(RenderStatus::Skipped) => {}
                Ok(RenderStatus::Reconfigured) => state.window.request_redraw(),
                Err(error) => {
                    eprintln!("w native render failed: {error}");
                    event_loop.exit();
                }
            },
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}

struct AppState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    engine: Engine<LocalGeometryBackendBridge>,
    renderer: MeshRenderer,
    depth_target: DepthTarget,
    demo_asset: DemoAsset,
    current_resource: String,
    selected_resource: String,
    resource_options: Vec<String>,
    last_load_error: Option<String>,
    orbit: OrbitCameraController,
    drag: DragState,
    egui_context: egui::Context,
    egui_state: EguiState,
    egui_painter: EguiPainter,
    rendered_frames: u32,
}

impl AppState {
    async fn new(event_loop: &ActiveEventLoop, resource: &str) -> Result<Self, NativeInitError> {
        let window = Arc::new(event_loop.create_window(window_attributes(resource))?);
        let size = clamp_surface_size(window.inner_size());
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .map_err(NativeInitError::from)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("w device"),
                ..Default::default()
            })
            .await
            .map_err(NativeInitError::from)?;
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .ok_or(NativeInitError::SurfaceUnsupported)?;
        config.width = size.width;
        config.height = size.height;
        surface.configure(&device, &config);

        let engine = Engine::new(
            LocalGeometryBackendBridge::default(),
            NullRenderBackend::default(),
        );
        let demo_asset = engine.build_demo_asset_for(resource)?;
        let camera = fit_camera_to_render_scene(&demo_asset.render_scene);
        let resource_options = available_local_resources();
        let defaults = RenderDefaults::default();
        let mut renderer = MeshRenderer::with_defaults(
            &device,
            config.format,
            ViewportSize::new(config.width, config.height),
            camera,
            defaults,
        );
        renderer.upload_prepared_scene(&device, &queue, &demo_asset.render_scene);
        let depth_target = DepthTarget::with_defaults(
            &device,
            ViewportSize::new(config.width, config.height),
            defaults,
            "w surface depth target",
        );
        let egui_context = egui::Context::default();
        configure_egui_style(&egui_context);
        let egui_state = EguiState::new(
            egui_context.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            Some(Theme::Dark),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_painter = EguiPainter::new(&device, config.format);

        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            engine,
            renderer,
            depth_target,
            demo_asset,
            current_resource: resource.to_string(),
            selected_resource: resource.to_string(),
            resource_options,
            last_load_error: None,
            orbit: OrbitCameraController::from_camera(camera),
            drag: DragState::default(),
            egui_context,
            egui_state,
            egui_painter,
            rendered_frames: 0,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }

        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth_target = DepthTarget::with_defaults(
            &self.device,
            ViewportSize::new(self.config.width, self.config.height),
            self.renderer.defaults(),
            "w surface depth target",
        );
        self.renderer.resize(
            &self.queue,
            ViewportSize::new(self.config.width, self.config.height),
        );
    }

    fn render(&mut self) -> Result<RenderStatus, NativeRenderError> {
        if self.config.width == 0 || self.config.height == 0 {
            return Ok(RenderStatus::Skipped);
        }

        let egui_frame = self.prepare_egui_frame();
        let free_textures = egui_frame.free_textures().to_vec();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.egui_painter.free_textures(&free_textures);
                return Ok(RenderStatus::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.egui_painter.free_textures(&free_textures);
                return Ok(RenderStatus::Reconfigured);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                self.egui_painter.free_textures(&free_textures);
                return Err(NativeRenderError::SurfaceValidation);
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("w frame encoder"),
            });
        self.renderer
            .render(&mut encoder, &view, self.depth_target.view());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("w egui pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            self.egui_painter.render(&mut pass, &egui_frame);
        }
        self.queue.submit([encoder.finish()]);
        frame.present();
        self.egui_painter.free_textures(&free_textures);

        Ok(RenderStatus::Ok)
    }

    fn on_window_event(&mut self, event: &WindowEvent) -> EguiEventResponse {
        let response = self.egui_state.on_window_event(&self.window, event);
        if response.repaint {
            self.window.request_redraw();
        }
        response
    }

    fn advance_frame(&mut self, auto_exit_after_frames: Option<u32>) -> bool {
        self.rendered_frames += 1;
        auto_exit_after_frames.is_some_and(|target| self.rendered_frames >= target)
    }

    fn begin_drag(&mut self) {
        self.drag.active = true;
        self.drag.anchored = false;
    }

    fn drag_to(&mut self, x: f32, y: f32) {
        if !self.drag.active {
            return;
        }

        if !self.drag.anchored {
            self.drag.last_x = x;
            self.drag.last_y = y;
            self.drag.anchored = true;
            return;
        }

        let dx = x - self.drag.last_x;
        let dy = y - self.drag.last_y;
        self.drag.last_x = x;
        self.drag.last_y = y;
        self.orbit.orbit_by_pixels(dx, dy);
        self.renderer.set_camera(&self.queue, self.orbit.camera());
    }

    fn end_drag(&mut self) {
        self.drag.active = false;
        self.drag.anchored = false;
    }

    fn zoom(&mut self, delta_y: f32) {
        self.orbit.zoom_by_wheel(delta_y);
        self.renderer.set_camera(&self.queue, self.orbit.camera());
    }

    fn prepare_egui_frame(&mut self) -> egui_painter::PreparedEguiFrame {
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut selected_resource = self.selected_resource.clone();
        let summary = self.demo_asset.summary_line();
        let load_error = self.last_load_error.clone();
        let camera = self.renderer.camera();
        let full_output = self.egui_context.run(raw_input, |context| {
            draw_native_toolbar(
                context,
                &mut selected_resource,
                &self.resource_options,
                &summary,
                load_error.as_deref(),
            );
            draw_world_axes_overlay(context, camera);
        });
        self.egui_state
            .handle_platform_output(&self.window, full_output.platform_output);

        if selected_resource != self.selected_resource {
            self.selected_resource = selected_resource.clone();
        }

        if selected_resource != self.current_resource {
            self.load_resource(&selected_resource);
        }

        let clipped_primitives = self
            .egui_context
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        self.egui_painter.prepare(
            &self.device,
            &self.queue,
            &full_output.textures_delta,
            &clipped_primitives,
            [self.config.width, self.config.height],
            full_output.pixels_per_point,
        )
    }

    fn load_resource(&mut self, resource: &str) {
        match self.engine.build_demo_asset_for(resource) {
            Ok(asset) => {
                let camera = fit_camera_to_render_scene(&asset.render_scene);
                self.renderer
                    .upload_prepared_scene(&self.device, &self.queue, &asset.render_scene);
                self.renderer.set_camera(&self.queue, camera);
                self.demo_asset = asset;
                self.current_resource = resource.to_string();
                self.selected_resource = resource.to_string();
                self.last_load_error = None;
                self.orbit = OrbitCameraController::from_camera(camera);
                self.window
                    .set_title(&format!("w native demo - {}", self.current_resource));
            }
            Err(error) => self.last_load_error = Some(error.to_string()),
        }
    }
}

#[derive(Debug)]
struct LocalGeometryBackendBridge {
    geometry_backend: GeometryBackend,
    ifc_artifacts_root: PathBuf,
}

impl Default for LocalGeometryBackendBridge {
    fn default() -> Self {
        Self {
            geometry_backend: GeometryBackend::default(),
            ifc_artifacts_root: default_ifc_artifacts_root(),
        }
    }
}

impl GeometryPackageSource for LocalGeometryBackendBridge {
    fn load_prepared_package(
        &self,
        resource: &str,
    ) -> Result<PreparedGeometryPackage, GeometryPackageSourceError> {
        if let Some(model_slug) = parse_ifc_body_resource(resource) {
            let available = available_local_resources_at(&self.ifc_artifacts_root);
            let canonical_resource = ifc_body_resource_name(model_slug);
            if !available
                .iter()
                .any(|candidate| candidate == &canonical_resource)
            {
                return Err(GeometryPackageSourceError::UnknownResource {
                    requested: resource.to_string(),
                    available: available.join(", "),
                });
            }

            let load = VelrIfcModel::load_body_package_with_cache_status_from_artifacts_root(
                &self.ifc_artifacts_root,
                model_slug,
            )
            .map_err(|error| GeometryPackageSourceError::LoadFailed(error.to_string()))?;
            println!(
                "w ifc geometry {} resource={} model={}",
                load.cache_status.as_str(),
                canonical_resource,
                model_slug
            );
            return Ok(load.package);
        }

        self.geometry_backend
            .build_demo_package_for(resource)
            .map_err(|error| map_geometry_backend_error(error, &self.ifc_artifacts_root))
    }
}

fn map_geometry_backend_error(
    error: GeometryBackendError,
    ifc_artifacts_root: &Path,
) -> GeometryPackageSourceError {
    match error {
        GeometryBackendError::Resource(ResourceError::UnknownResource { requested, .. }) => {
            GeometryPackageSourceError::UnknownResource {
                requested,
                available: available_local_resources_at(ifc_artifacts_root).join(", "),
            }
        }
        other => GeometryPackageSourceError::LoadFailed(other.to_string()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderStatus {
    Ok,
    Reconfigured,
    Skipped,
}

#[derive(Debug, Error)]
enum NativeInitError {
    #[error(transparent)]
    Window(#[from] winit::error::OsError),
    #[error(transparent)]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error(transparent)]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error(transparent)]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error(transparent)]
    Device(#[from] wgpu::RequestDeviceError),
    #[error(transparent)]
    Runtime(#[from] cc_w_runtime::RuntimeError),
    #[error("the window surface is not supported by the selected adapter")]
    SurfaceUnsupported,
}

#[derive(Debug, Error)]
enum NativeRenderError {
    #[error("wgpu surface validation failed while acquiring the next frame")]
    SurfaceValidation,
}

#[derive(Clone, Copy, Debug, Default)]
struct DragState {
    active: bool,
    anchored: bool,
    last_x: f32,
    last_y: f32,
}

#[derive(Clone, Copy, Debug)]
struct OrbitCameraController {
    target: DVec3,
    radius: f64,
    yaw_radians: f64,
    pitch_radians: f64,
    vertical_fov_degrees: f64,
    near_plane: f64,
    far_plane: f64,
}

impl OrbitCameraController {
    fn from_camera(camera: Camera) -> Self {
        let offset = camera.eye - camera.target;
        let radius = offset.length().max(0.25);
        let pitch_radians = (offset.z / radius).clamp(-1.0, 1.0).asin();
        let yaw_radians = offset.x.atan2(-offset.y);

        Self {
            target: camera.target,
            radius,
            yaw_radians,
            pitch_radians,
            vertical_fov_degrees: camera.vertical_fov_degrees,
            near_plane: camera.near_plane,
            far_plane: camera.far_plane,
        }
    }

    fn camera(&self) -> Camera {
        let cos_pitch = self.pitch_radians.cos();
        let offset = DVec3::new(
            self.yaw_radians.sin() * cos_pitch,
            -self.yaw_radians.cos() * cos_pitch,
            self.pitch_radians.sin(),
        ) * self.radius;

        Camera {
            eye: self.target + offset,
            target: self.target,
            up: WORLD_UP,
            vertical_fov_degrees: self.vertical_fov_degrees,
            near_plane: self.near_plane,
            far_plane: self.far_plane.max(self.radius * 8.0),
        }
    }

    fn orbit_by_pixels(&mut self, dx: f32, dy: f32) {
        const ORBIT_SENSITIVITY: f64 = 0.01;
        const MAX_PITCH: f64 = 1.52;

        self.yaw_radians -= f64::from(dx) * ORBIT_SENSITIVITY;
        self.pitch_radians =
            (self.pitch_radians + (f64::from(dy) * ORBIT_SENSITIVITY)).clamp(-MAX_PITCH, MAX_PITCH);
    }

    fn zoom_by_wheel(&mut self, delta_y: f32) {
        let scale = (f64::from(delta_y) * 0.0015).exp();
        self.radius = (self.radius * scale).clamp(0.2, 500.0);
    }
}

fn mouse_wheel_delta_y(delta: MouseScrollDelta) -> f32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => y * 40.0,
        MouseScrollDelta::PixelDelta(position) => position.y as f32,
    }
}

fn window_attributes(resource: &str) -> WindowAttributes {
    Window::default_attributes()
        .with_title(format!("w native demo - {resource}"))
        .with_inner_size(PhysicalSize::new(1280_u32, 720_u32))
}

fn configure_egui_style(context: &egui::Context) {
    let bg = Color32::from_rgb(14, 18, 24);
    let panel = Color32::from_rgb(22, 27, 35);
    let widget = Color32::from_rgb(30, 36, 46);
    let widget_hover = Color32::from_rgb(41, 49, 62);
    let accent = Color32::from_rgb(230, 93, 71);
    let accent_hover = Color32::from_rgb(244, 111, 89);
    let text = Color32::from_rgb(241, 244, 250);
    let muted = Color32::from_rgba_unmultiplied(241, 244, 250, 176);

    let mut style = (*context.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.override_text_color = Some(text);
    style.visuals.panel_fill = panel;
    style.visuals.window_fill = panel;
    style.visuals.faint_bg_color = bg;
    style.visuals.extreme_bg_color = bg;
    style.visuals.code_bg_color = bg;
    style.visuals.selection.bg_fill = accent;
    style.visuals.selection.stroke = egui::Stroke::new(1.0, accent_hover);
    style.visuals.widgets.noninteractive.bg_fill = panel;
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, muted);
    style.visuals.widgets.inactive.bg_fill = widget;
    style.visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, text);
    style.visuals.widgets.hovered.bg_fill = widget_hover;
    style.visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, text);
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, accent_hover);
    style.visuals.widgets.active.bg_fill = accent;
    style.visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, text);
    style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, accent_hover);
    style.spacing.item_spacing = egui::vec2(14.0, 10.0);
    style.spacing.button_padding = egui::vec2(10.0, 7.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.combo_width = 260.0;
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(18.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        FontId::new(12.0, FontFamily::Proportional),
    );
    context.set_style(style);
}

fn draw_native_toolbar(
    context: &egui::Context,
    selected_resource: &mut String,
    resource_options: &[String],
    summary: &str,
    load_error: Option<&str>,
) {
    let accent = Color32::from_rgb(230, 93, 71);
    let muted = Color32::from_rgba_unmultiplied(241, 244, 250, 176);
    let error_color = Color32::from_rgb(255, 143, 143);
    let status_text = load_error.unwrap_or(summary);
    let status_color = if load_error.is_some() {
        error_color
    } else {
        muted
    };

    TopBottomPanel::top("w-native-toolbar")
        .resizable(false)
        .exact_height(56.0)
        .show(context, |ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.label(RichText::new("w").heading().strong().color(accent));
                ui.label(RichText::new("Resource").strong());
                ComboBox::from_id_salt("resource-picker")
                    .selected_text(friendly_resource_label(selected_resource))
                    .width(280.0)
                    .show_ui(ui, |ui| {
                        for resource in resource_options {
                            ui.selectable_value(
                                selected_resource,
                                resource.to_string(),
                                friendly_resource_label(resource),
                            );
                        }
                    });
                ui.label(RichText::new("Drag to orbit. Wheel to zoom.").color(muted));
                ui.separator();
                let status_width = ui.available_width().max(120.0);
                ui.add_sized(
                    [status_width, 18.0],
                    egui::Label::new(RichText::new(status_text).color(status_color)).truncate(),
                );
            });
        });
}

fn friendly_resource_label(resource: &str) -> String {
    match resource {
        "demo/mapped-pentagon-pair" => "mapped-pentagon-pair (per-instance color)".to_string(),
        _ if resource.starts_with("demo/") => resource.trim_start_matches("demo/").to_string(),
        _ if resource.starts_with("ifc/") => {
            format!("{} (ifc)", resource.trim_start_matches("ifc/"))
        }
        _ => resource.to_string(),
    }
}

#[derive(Clone, Copy)]
struct OverlayAxis {
    label: &'static str,
    color: Color32,
    direction: Vec2,
    depth: f32,
}

fn draw_world_axes_overlay(context: &egui::Context, camera: Camera) {
    let content_rect = context.content_rect();
    let radius = 34.0;
    let margin = 16.0;
    let left_offset = -5.0;
    let bottom_offset = 7.0;
    let center = Pos2::new(
        content_rect.left() + margin + radius + left_offset,
        content_rect.bottom() - margin - radius + bottom_offset,
    );
    let painter = context.layer_painter(egui::LayerId::new(
        Order::Foreground,
        egui::Id::new("w-world-axes-overlay"),
    ));
    let axis_length = 24.0;
    let origin_fill = Color32::from_rgba_unmultiplied(241, 244, 250, 208);

    painter.circle_filled(center, 3.5, origin_fill);

    let mut axes = [
        overlay_axis(camera, WORLD_RIGHT, "X", Color32::from_rgb(238, 99, 82)),
        overlay_axis(camera, WORLD_FORWARD, "Y", Color32::from_rgb(102, 214, 166)),
        overlay_axis(camera, WORLD_UP, "Z", Color32::from_rgb(110, 168, 254)),
    ];
    axes.sort_by(|left, right| right.depth.total_cmp(&left.depth));

    for axis in axes {
        let vector = axis.direction * axis_length;
        painter.arrow(center, vector, Stroke::new(2.25, axis.color));

        let label_anchor = center + vector;
        let label_offset = if axis.direction.length_sq() > 1.0e-6 {
            axis.direction.normalized() * 12.0
        } else {
            Vec2::new(0.0, -12.0)
        };
        painter.text(
            label_anchor + label_offset,
            Align2::CENTER_CENTER,
            axis.label,
            FontId::new(12.0, FontFamily::Proportional),
            axis.color,
        );
    }
}

fn overlay_axis(camera: Camera, axis: DVec3, label: &'static str, color: Color32) -> OverlayAxis {
    let camera_space = camera.view_from_world().transform_vector3(axis);

    OverlayAxis {
        label,
        color,
        direction: Vec2::new(camera_space.x as f32, -(camera_space.y as f32)),
        depth: camera_space.z as f32,
    }
}

fn clamp_surface_size(size: PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(size.width.max(1), size.height.max(1))
}

fn auto_exit_after_frames() -> Option<u32> {
    std::env::var("CC_W_AUTO_EXIT_FRAMES")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
}

impl NativeLaunchOptions {
    fn from_env_args() -> Result<Self, LaunchControlFlow> {
        let default_resource = std::env::var("CC_W_DEMO_RESOURCE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_DEMO_RESOURCE.to_string());
        Self::parse_args_with_default(default_resource, std::env::args())
    }

    #[cfg(test)]
    fn parse_from<I, S>(args: I) -> Result<Self, LaunchControlFlow>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::parse_args_with_default(DEFAULT_DEMO_RESOURCE.to_string(), args)
    }

    fn parse_args_with_default<I, S>(
        default_resource: String,
        args: I,
    ) -> Result<Self, LaunchControlFlow>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut resource = default_resource;
        let mut args = args.into_iter().map(|arg| arg.as_ref().to_string());
        let _program = args.next();

        while let Some(argument) = args.next() {
            match argument.as_str() {
                "-h" | "--help" => return Err(LaunchControlFlow::ExitSuccess(native_usage())),
                "--list-resources" => {
                    return Err(LaunchControlFlow::ExitSuccess(
                        available_local_resources().join("\n"),
                    ));
                }
                "-r" | "--resource" => {
                    let Some(value) = args.next() else {
                        return Err(LaunchControlFlow::ExitFailure(
                            "missing value for `--resource`".to_string(),
                        ));
                    };
                    resource = value;
                }
                other => {
                    return Err(LaunchControlFlow::ExitFailure(format!(
                        "unknown argument `{other}`\n\n{}",
                        native_usage()
                    )));
                }
            }
        }

        Ok(Self {
            resource,
            auto_exit_after_frames: auto_exit_after_frames(),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LaunchControlFlow {
    ExitSuccess(String),
    ExitFailure(String),
}

fn native_usage() -> String {
    format!(
        "\
Usage:
  cargo run -p cc-w-platform-native -- [--resource {default_resource}]
  cargo run -p cc-w-platform-native -- --list-resources

Environment:
  CC_W_DEMO_RESOURCE sets the initial resource when --resource is omitted.
  CC_W_AUTO_EXIT_FRAMES exits after N rendered frames.

Resources:
  {resources}
",
        default_resource = DEFAULT_DEMO_RESOURCE,
        resources = available_local_resources().join("\n  "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_launch_options_use_default_resource() {
        let options = NativeLaunchOptions::parse_from(["cc-w-platform-native"]).expect("options");

        assert_eq!(options.resource, DEFAULT_DEMO_RESOURCE);
    }

    #[test]
    fn native_launch_options_accept_resource_argument() {
        let options = NativeLaunchOptions::parse_from([
            "cc-w-platform-native",
            "--resource",
            "demo/mapped-pentagon-pair",
        ])
        .expect("options");

        assert_eq!(options.resource, "demo/mapped-pentagon-pair");
    }

    #[test]
    fn native_launch_options_accept_ifc_resource_argument() {
        let options = NativeLaunchOptions::parse_from([
            "cc-w-platform-native",
            "--resource",
            "ifc/building-architecture",
        ])
        .expect("options");

        assert_eq!(options.resource, "ifc/building-architecture");
    }

    #[test]
    fn native_launch_options_list_resources_exits_successfully() {
        let control = NativeLaunchOptions::parse_from(["cc-w-platform-native", "--list-resources"])
            .expect_err("control flow");

        assert!(matches!(control, LaunchControlFlow::ExitSuccess(_)));
    }

    #[test]
    fn native_launch_options_reject_unknown_argument() {
        let control =
            NativeLaunchOptions::parse_from(["cc-w-platform-native", "--wat"]).expect_err("error");

        assert!(matches!(control, LaunchControlFlow::ExitFailure(_)));
    }

    #[test]
    fn orbit_controller_orbits_around_same_target() {
        let camera = fit_camera_to_render_scene_for_tests();
        let mut orbit = OrbitCameraController::from_camera(camera);

        orbit.orbit_by_pixels(120.0, -45.0);
        let orbited = orbit.camera();

        assert_eq!(orbited.target, camera.target);
        assert!(orbited.eye.distance(camera.eye) > 0.1);
    }

    #[test]
    fn orbit_controller_zoom_changes_eye_distance() {
        let camera = fit_camera_to_render_scene_for_tests();
        let mut orbit = OrbitCameraController::from_camera(camera);
        let before = orbit.camera().eye.distance(orbit.camera().target);

        orbit.zoom_by_wheel(-120.0);
        let after = orbit.camera().eye.distance(orbit.camera().target);

        assert!(after < before);
    }

    fn fit_camera_to_render_scene_for_tests() -> Camera {
        Camera {
            eye: DVec3::new(2.0, -4.0, 3.0),
            target: DVec3::new(0.5, 0.0, 1.0),
            up: WORLD_UP,
            vertical_fov_degrees: 45.0,
            near_plane: 0.1,
            far_plane: 100.0,
        }
    }
}
