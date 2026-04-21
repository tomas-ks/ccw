use cc_w_backend::{GeometryBackend, GeometryBackendError, ResourceError};
use cc_w_render::NullRenderBackend;
#[cfg(target_arch = "wasm32")]
use cc_w_runtime::RuntimeSceneState;
use cc_w_runtime::{DemoAsset, Engine, GeometryPackageSource, GeometryPackageSourceError};
use cc_w_types::{
    Bounds3, DefaultRenderClass, DisplayColor, ExternalId, GeometryDefinitionId,
    GeometryInstanceId, PreparedGeometryDefinition, PreparedGeometryElement,
    PreparedGeometryInstance, PreparedGeometryPackage, PreparedMesh, PreparedVertex,
    SemanticElementId,
};
use serde::{Deserialize, Serialize};

const DEFAULT_WEB_RESOURCE: &str = "demo/revolved-solid";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebResourceCatalog {
    pub resources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebPreparedPackageResponse {
    pub resource: String,
    pub package: WebPreparedGeometryPackage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedGeometryPackage {
    pub definitions: Vec<WebPreparedGeometryDefinition>,
    pub elements: Vec<WebPreparedGeometryElement>,
    pub instances: Vec<WebPreparedGeometryInstance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedGeometryDefinition {
    pub id: u64,
    pub mesh: WebPreparedMesh,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedGeometryElement {
    pub id: String,
    pub label: String,
    pub declared_entity: String,
    pub default_render_class: String,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedGeometryInstance {
    pub id: u64,
    pub element_id: String,
    pub definition_id: u64,
    pub transform: [f64; 16],
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub external_id: String,
    pub label: String,
    pub display_color: Option<[f32; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedMesh {
    pub local_origin: [f64; 3],
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub vertices: Vec<WebPreparedVertex>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebPreparedVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

pub fn demo_summary_string() -> String {
    match build_demo_asset(DEFAULT_WEB_RESOURCE) {
        Ok(asset) => asset.summary_line(),
        Err(error) => format!("w web demo failed: {error}"),
    }
}

fn build_demo_asset(resource: &str) -> Result<DemoAsset, cc_w_runtime::RuntimeError> {
    let engine = Engine::new(
        LocalGeometryBackendBridge::default(),
        NullRenderBackend::default(),
    );
    engine.build_demo_asset_for(resource)
}

#[cfg(target_arch = "wasm32")]
fn build_runtime_scene(resource: &str) -> Result<RuntimeSceneState, cc_w_runtime::RuntimeError> {
    let engine = Engine::new(
        LocalGeometryBackendBridge::default(),
        NullRenderBackend::default(),
    );
    engine.build_runtime_scene_for(resource)
}

impl WebPreparedGeometryPackage {
    pub fn from_prepared_package(package: &PreparedGeometryPackage) -> Self {
        Self {
            definitions: package
                .definitions
                .iter()
                .map(WebPreparedGeometryDefinition::from_prepared_definition)
                .collect(),
            elements: package
                .elements
                .iter()
                .map(WebPreparedGeometryElement::from_prepared_element)
                .collect(),
            instances: package
                .instances
                .iter()
                .map(WebPreparedGeometryInstance::from_prepared_instance)
                .collect(),
        }
    }

    pub fn try_into_prepared_package(self) -> Result<PreparedGeometryPackage, String> {
        Ok(PreparedGeometryPackage {
            definitions: self
                .definitions
                .into_iter()
                .map(WebPreparedGeometryDefinition::into_prepared_definition)
                .collect(),
            elements: self
                .elements
                .into_iter()
                .map(WebPreparedGeometryElement::try_into_prepared_element)
                .collect::<Result<Vec<_>, _>>()?,
            instances: self
                .instances
                .into_iter()
                .map(WebPreparedGeometryInstance::into_prepared_instance)
                .collect(),
        })
    }
}

impl WebPreparedGeometryDefinition {
    fn from_prepared_definition(definition: &PreparedGeometryDefinition) -> Self {
        Self {
            id: definition.id.0,
            mesh: WebPreparedMesh::from_prepared_mesh(&definition.mesh),
        }
    }

    fn into_prepared_definition(self) -> PreparedGeometryDefinition {
        PreparedGeometryDefinition {
            id: GeometryDefinitionId(self.id),
            mesh: self.mesh.into_prepared_mesh(),
        }
    }
}

impl WebPreparedGeometryElement {
    fn from_prepared_element(element: &PreparedGeometryElement) -> Self {
        Self {
            id: element.id.as_str().to_string(),
            label: element.label.clone(),
            declared_entity: element.declared_entity.clone(),
            default_render_class: default_render_class_name(element.default_render_class)
                .to_string(),
            bounds_min: element.bounds.min.to_array(),
            bounds_max: element.bounds.max.to_array(),
        }
    }

    fn try_into_prepared_element(self) -> Result<PreparedGeometryElement, String> {
        Ok(PreparedGeometryElement {
            id: SemanticElementId::new(self.id),
            label: self.label,
            declared_entity: self.declared_entity,
            default_render_class: parse_default_render_class(&self.default_render_class)?,
            bounds: Bounds3 {
                min: glam::DVec3::from_array(self.bounds_min),
                max: glam::DVec3::from_array(self.bounds_max),
            },
        })
    }
}

impl WebPreparedGeometryInstance {
    fn from_prepared_instance(instance: &PreparedGeometryInstance) -> Self {
        Self {
            id: instance.id.0,
            element_id: instance.element_id.as_str().to_string(),
            definition_id: instance.definition_id.0,
            transform: instance.transform.to_cols_array(),
            bounds_min: instance.bounds.min.to_array(),
            bounds_max: instance.bounds.max.to_array(),
            external_id: instance.external_id.as_str().to_string(),
            label: instance.label.clone(),
            display_color: instance.display_color.map(DisplayColor::as_rgb),
        }
    }

    fn into_prepared_instance(self) -> PreparedGeometryInstance {
        PreparedGeometryInstance {
            id: GeometryInstanceId(self.id),
            element_id: SemanticElementId::new(self.element_id),
            definition_id: GeometryDefinitionId(self.definition_id),
            transform: glam::DMat4::from_cols_array(&self.transform),
            bounds: Bounds3 {
                min: glam::DVec3::from_array(self.bounds_min),
                max: glam::DVec3::from_array(self.bounds_max),
            },
            external_id: ExternalId::new(self.external_id),
            label: self.label,
            display_color: self.display_color.map(|rgb| DisplayColor { rgb }),
        }
    }
}

impl WebPreparedMesh {
    fn from_prepared_mesh(mesh: &PreparedMesh) -> Self {
        Self {
            local_origin: mesh.local_origin.to_array(),
            bounds_min: mesh.bounds.min.to_array(),
            bounds_max: mesh.bounds.max.to_array(),
            vertices: mesh
                .vertices
                .iter()
                .copied()
                .map(WebPreparedVertex::from_prepared_vertex)
                .collect(),
            indices: mesh.indices.clone(),
        }
    }

    fn into_prepared_mesh(self) -> PreparedMesh {
        PreparedMesh {
            local_origin: glam::DVec3::from_array(self.local_origin),
            bounds: Bounds3 {
                min: glam::DVec3::from_array(self.bounds_min),
                max: glam::DVec3::from_array(self.bounds_max),
            },
            vertices: self
                .vertices
                .into_iter()
                .map(WebPreparedVertex::into_prepared_vertex)
                .collect(),
            indices: self.indices,
        }
    }
}

impl WebPreparedVertex {
    fn from_prepared_vertex(vertex: PreparedVertex) -> Self {
        Self {
            position: vertex.position,
            normal: vertex.normal,
        }
    }

    fn into_prepared_vertex(self) -> PreparedVertex {
        PreparedVertex {
            position: self.position,
            normal: self.normal,
        }
    }
}

fn default_render_class_name(class: DefaultRenderClass) -> &'static str {
    match class {
        DefaultRenderClass::Physical => "physical",
        DefaultRenderClass::Space => "space",
        DefaultRenderClass::Zone => "zone",
        DefaultRenderClass::Helper => "helper",
        DefaultRenderClass::Other => "other",
    }
}

fn parse_default_render_class(name: &str) -> Result<DefaultRenderClass, String> {
    match name {
        "physical" => Ok(DefaultRenderClass::Physical),
        "space" => Ok(DefaultRenderClass::Space),
        "zone" => Ok(DefaultRenderClass::Zone),
        "helper" => Ok(DefaultRenderClass::Helper),
        "other" => Ok(DefaultRenderClass::Other),
        other => Err(format!("unknown default render class `{other}`")),
    }
}

#[derive(Debug, Default)]
struct LocalGeometryBackendBridge {
    geometry_backend: GeometryBackend,
}

impl GeometryPackageSource for LocalGeometryBackendBridge {
    fn load_prepared_package(
        &self,
        resource: &str,
    ) -> Result<PreparedGeometryPackage, GeometryPackageSourceError> {
        self.geometry_backend
            .build_demo_package_for(resource)
            .map_err(map_geometry_backend_error)
    }
}

fn map_geometry_backend_error(error: GeometryBackendError) -> GeometryPackageSourceError {
    match error {
        GeometryBackendError::Resource(ResourceError::UnknownResource {
            requested,
            available,
        }) => GeometryPackageSourceError::UnknownResource {
            requested,
            available,
        },
        other => GeometryPackageSourceError::LoadFailed(other.to_string()),
    }
}

#[cfg(target_arch = "wasm32")]
use cc_w_backend::available_demo_resources;
#[cfg(target_arch = "wasm32")]
use cc_w_render::{Camera, DepthTarget, MeshRenderer, ViewportSize, fit_camera_to_render_scene};
#[cfg(target_arch = "wasm32")]
use cc_w_types::{PreparedRenderScene, WORLD_FORWARD, WORLD_RIGHT, WORLD_UP};
#[cfg(target_arch = "wasm32")]
use glam::{DVec3, Vec2};
#[cfg(target_arch = "wasm32")]
use js_sys::{Array, decode_uri_component};
#[cfg(target_arch = "wasm32")]
use std::{cell::RefCell, rc::Rc};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{JsCast, closure::Closure, prelude::*};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::{JsFuture, spawn_local};
#[cfg(target_arch = "wasm32")]
use web_sys::{
    CanvasRenderingContext2d, Document, Element, Event, HtmlCanvasElement, HtmlElement,
    HtmlSelectElement, MouseEvent, RequestInit, Response, WheelEvent, Window,
};

#[cfg(target_arch = "wasm32")]
#[derive(Serialize)]
struct WebPackageRequest<'a> {
    resource: &'a str,
}

#[cfg(target_arch = "wasm32")]
fn fallback_demo_resources() -> Vec<String> {
    available_demo_resources()
        .into_iter()
        .map(|resource| resource.to_string())
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn is_demo_resource(resource: &str) -> bool {
    available_demo_resources()
        .iter()
        .any(|candidate| *candidate == resource)
}

#[cfg(target_arch = "wasm32")]
fn is_file_protocol(window: &Window) -> bool {
    window.location().protocol().ok().as_deref() == Some("file:")
}

#[cfg(target_arch = "wasm32")]
async fn fetch_available_resources(window: &Window) -> Result<Vec<String>, String> {
    if is_file_protocol(window) {
        return Ok(fallback_demo_resources());
    }

    let text = fetch_server_text(window, "/api/resources", "GET", None).await?;
    let catalog: WebResourceCatalog = serde_json::from_str(&text)
        .map_err(|error| format!("invalid /api/resources JSON: {error}"))?;
    if catalog.resources.is_empty() {
        return Err("server returned an empty resource catalog".to_string());
    }
    Ok(catalog.resources)
}

#[cfg(target_arch = "wasm32")]
async fn fetch_runtime_scene_from_server(
    window: &Window,
    resource: &str,
) -> Result<RuntimeSceneState, String> {
    let request_text = serde_json::to_string(&WebPackageRequest { resource })
        .map_err(|error| format!("failed to encode package request JSON: {error}"))?;
    let text = fetch_server_text(window, "/api/package", "POST", Some(&request_text)).await?;
    let response: WebPreparedPackageResponse = serde_json::from_str(&text)
        .map_err(|error| format!("invalid /api/package JSON: {error}"))?;
    let package = response.package.try_into_prepared_package()?;
    RuntimeSceneState::from_prepared_package(package).map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn fetch_runtime_scene(window: &Window, resource: &str) -> Result<RuntimeSceneState, String> {
    if !is_file_protocol(window) {
        match fetch_runtime_scene_from_server(window, resource).await {
            Ok(scene) => return Ok(scene),
            Err(error) if !is_demo_resource(resource) => return Err(error),
            Err(error) => log_viewer_error(&format!(
                "w web viewer package fetch fell back to local demo path for `{resource}`: {error}"
            )),
        }
    }

    build_runtime_scene(resource).map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
async fn fetch_server_text(
    window: &Window,
    url: &str,
    method: &str,
    body: Option<&str>,
) -> Result<String, String> {
    let response_value = if method == "GET" {
        JsFuture::from(window.fetch_with_str(url))
            .await
            .map_err(js_error)?
    } else {
        let init = RequestInit::new();
        init.set_method(method);
        if let Some(body) = body {
            init.set_body(&JsValue::from_str(body));
        }
        JsFuture::from(window.fetch_with_str_and_init(url, &init))
            .await
            .map_err(js_error)?
    };

    let response: Response = response_value
        .dyn_into()
        .map_err(|_| "w web viewer fetch returned a non-response value".to_string())?;
    let text_promise = response.text().map_err(js_error)?;
    let text = JsFuture::from(text_promise)
        .await
        .map_err(js_error)?
        .as_string()
        .unwrap_or_default();
    if !response.ok() {
        return Err(if text.is_empty() {
            format!(
                "server request to `{url}` failed with {}",
                response.status()
            )
        } else {
            text
        });
    }

    Ok(text)
}

#[cfg(target_arch = "wasm32")]
async fn load_resource_into_state(
    state: Rc<RefCell<WebViewerState>>,
    resource: String,
) -> Result<(), String> {
    let window = {
        let mut state = state.borrow_mut();
        state.begin_resource_load(&resource);
        state.window.clone()
    };
    let runtime_scene = match fetch_runtime_scene(&window, &resource).await {
        Ok(runtime_scene) => runtime_scene,
        Err(error) => {
            state
                .borrow_mut()
                .finish_resource_load_failed(&resource, &error);
            return Err(error);
        }
    };
    let mut state = state.borrow_mut();
    state.apply_runtime_scene(resource, runtime_scene);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static WEB_VIEWER_APP: RefCell<Option<WebViewerApp>> = const { RefCell::new(None) };
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn demo_summary() -> String {
    demo_summary_string()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_current_resource() -> Result<String, JsValue> {
    with_web_viewer_state(|state| Ok(state.current_resource.clone()))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_list_element_ids() -> Result<Array, JsValue> {
    with_web_viewer_state(|state| {
        Ok(semantic_ids_to_array(
            state
                .runtime_scene
                .package()
                .elements
                .iter()
                .map(|element| &element.id),
        ))
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_visible_element_ids() -> Result<Array, JsValue> {
    with_web_viewer_state(|state| {
        let ids = state.runtime_scene.visible_element_ids();
        Ok(semantic_ids_to_array(ids.iter()))
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_selected_element_ids() -> Result<Array, JsValue> {
    with_web_viewer_state(|state| {
        let ids = state.runtime_scene.selected_element_ids();
        Ok(semantic_ids_to_array(ids.iter()))
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_hide_elements(ids: Array) -> Result<u32, JsValue> {
    with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.hide_elements(ids.iter()) as u32;
        state.upload_runtime_scene(false);
        Ok(changed)
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_show_elements(ids: Array) -> Result<u32, JsValue> {
    with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.show_elements(ids.iter()) as u32;
        state.upload_runtime_scene(false);
        Ok(changed)
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_reset_element_visibility(ids: Array) -> Result<u32, JsValue> {
    with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.reset_visibility(ids.iter()) as u32;
        state.upload_runtime_scene(false);
        Ok(changed)
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_reset_all_visibility() -> Result<u32, JsValue> {
    with_web_viewer_state_mut(|state| {
        let ids = state
            .runtime_scene
            .package()
            .elements
            .iter()
            .map(|element| element.id.clone())
            .collect::<Vec<_>>();
        let changed = state.runtime_scene.reset_visibility(ids.iter()) as u32;
        state.upload_runtime_scene(false);
        Ok(changed)
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_select_elements(ids: Array) -> Result<u32, JsValue> {
    with_web_viewer_state_mut(|state| {
        let ids = semantic_ids_from_array(&ids)?;
        let changed = state.runtime_scene.select_elements(ids.iter()) as u32;
        state.refresh_status();
        Ok(changed)
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_clear_selection() -> Result<u32, JsValue> {
    with_web_viewer_state_mut(|state| {
        let changed = state.runtime_scene.clear_selection() as u32;
        state.refresh_status();
        Ok(changed)
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_frame_visible() -> Result<(), JsValue> {
    with_web_viewer_state_mut(|state| {
        state.frame_visible_scene();
        Ok(())
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_resize_viewport() -> Result<(), JsValue> {
    with_web_viewer_state_mut(|state| {
        state.resize_to_window()?;
        state.render()?;
        Ok(())
    })
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    spawn_local(async {
        match WebViewerApp::new().await {
            Ok(app) => {
                WEB_VIEWER_APP.with(|slot| {
                    *slot.borrow_mut() = Some(app);
                });
            }
            Err(error) => log_viewer_error(&error),
        }
    });
}

#[cfg(target_arch = "wasm32")]
struct WebViewerApp {
    _state: Rc<RefCell<WebViewerState>>,
    _resource_change: Closure<dyn FnMut(Event)>,
    _mouse_down: Closure<dyn FnMut(MouseEvent)>,
    _mouse_move: Closure<dyn FnMut(MouseEvent)>,
    _mouse_up: Closure<dyn FnMut(MouseEvent)>,
    _mouse_leave: Closure<dyn FnMut(MouseEvent)>,
    _wheel: Closure<dyn FnMut(WheelEvent)>,
    _resize: Closure<dyn FnMut(Event)>,
    _animation_frame: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>,
}

#[cfg(target_arch = "wasm32")]
impl WebViewerApp {
    async fn new() -> Result<Self, String> {
        let window = web_sys::window().ok_or("w web viewer could not access `window`")?;
        let document = window
            .document()
            .ok_or("w web viewer could not access `document`")?;
        let canvas = typed_element::<HtmlCanvasElement>(&document, "viewer-canvas")?;
        let axes_overlay = typed_element::<HtmlCanvasElement>(&document, "axes-overlay")?;
        let resource_picker = typed_element::<HtmlSelectElement>(&document, "resource-picker")?;
        let status_line = typed_element::<HtmlElement>(&document, "status-line")?;
        let resources = match fetch_available_resources(&window).await {
            Ok(resources) => resources,
            Err(error) => {
                log_viewer_error(&format!(
                    "w web viewer resource catalog fell back to demo-only resources: {error}"
                ));
                fallback_demo_resources()
            }
        };
        populate_resource_picker(&resource_picker, &resources);
        let initial_resource = initial_web_resource(&window, &resources);
        resource_picker.set_value(&initial_resource);

        let axes_overlay_context = axes_overlay_context(&axes_overlay)?;
        let (width, height) = resize_canvases_to_window(&window, &canvas, &axes_overlay)?;
        let runtime_scene = fetch_runtime_scene(&window, &initial_resource).await?;
        let render_scene = runtime_scene.compose_render_scene();
        let camera = fit_camera_to_render_scene(&render_scene);
        let orbit = OrbitCameraController::from_camera(camera);

        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|error| format!("w web viewer could not create a GPU surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .map_err(|error| format!("w web viewer could not request an adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("w web device"),
                ..Default::default()
            })
            .await
            .map_err(|error| format!("w web viewer could not request a device: {error}"))?;
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or("w web viewer could not determine a default surface configuration")?;
        config.width = width;
        config.height = height;
        surface.configure(&device, &config);

        let defaults = cc_w_render::RenderDefaults::default();
        let mut renderer = MeshRenderer::with_defaults(
            &device,
            config.format,
            ViewportSize::new(config.width, config.height),
            camera,
            defaults,
        );
        renderer.upload_prepared_scene(&device, &queue, &render_scene);
        let depth_target = DepthTarget::with_defaults(
            &device,
            ViewportSize::new(config.width, config.height),
            defaults,
            "w web depth target",
        );

        let state = Rc::new(RefCell::new(WebViewerState {
            window: window.clone(),
            canvas: canvas.clone(),
            axes_overlay,
            axes_overlay_context,
            resource_picker: resource_picker.clone(),
            status_line,
            current_resource: initial_resource.clone(),
            runtime_scene,
            surface,
            device,
            queue,
            config,
            renderer,
            depth_target,
            orbit,
            drag: DragState::default(),
        }));
        state.borrow().refresh_status();

        let resource_state = state.clone();
        let resource_change = Closure::wrap(Box::new(move |_event: Event| {
            let resource = resource_state.borrow().resource_picker.value();
            let resource_state = resource_state.clone();
            spawn_local(async move {
                if let Err(error) = load_resource_into_state(resource_state, resource).await {
                    log_viewer_error(&error);
                }
            });
        }) as Box<dyn FnMut(Event)>);
        resource_picker
            .add_event_listener_with_callback("change", resource_change.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let mouse_down_state = state.clone();
        let mouse_down = Closure::wrap(Box::new(move |event: MouseEvent| {
            mouse_down_state
                .borrow_mut()
                .begin_drag(event.client_x() as f32, event.client_y() as f32);
        }) as Box<dyn FnMut(MouseEvent)>);
        canvas
            .add_event_listener_with_callback("mousedown", mouse_down.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let mouse_move_state = state.clone();
        let mouse_move = Closure::wrap(Box::new(move |event: MouseEvent| {
            if let Err(error) = mouse_move_state
                .borrow_mut()
                .drag_to(event.client_x() as f32, event.client_y() as f32)
            {
                log_viewer_error(&error);
            }
        }) as Box<dyn FnMut(MouseEvent)>);
        window
            .add_event_listener_with_callback("mousemove", mouse_move.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let mouse_up_state = state.clone();
        let mouse_up = Closure::wrap(Box::new(move |_event: MouseEvent| {
            mouse_up_state.borrow_mut().end_drag();
        }) as Box<dyn FnMut(MouseEvent)>);
        window
            .add_event_listener_with_callback("mouseup", mouse_up.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let mouse_leave_state = state.clone();
        let mouse_leave = Closure::wrap(Box::new(move |_event: MouseEvent| {
            mouse_leave_state.borrow_mut().end_drag();
        }) as Box<dyn FnMut(MouseEvent)>);
        canvas
            .add_event_listener_with_callback("mouseleave", mouse_leave.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let wheel_state = state.clone();
        let wheel = Closure::wrap(Box::new(move |event: WheelEvent| {
            event.prevent_default();
            if let Err(error) = wheel_state.borrow_mut().zoom(event.delta_y() as f32) {
                log_viewer_error(&error);
            }
        }) as Box<dyn FnMut(WheelEvent)>);
        canvas
            .add_event_listener_with_callback("wheel", wheel.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let resize_state = state.clone();
        let resize = Closure::wrap(Box::new(move |_event: Event| {
            if let Err(error) = resize_state.borrow_mut().resize_to_window() {
                log_viewer_error(&error);
            }
        }) as Box<dyn FnMut(Event)>);
        window
            .add_event_listener_with_callback("resize", resize.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let animation_frame: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> =
            Rc::new(RefCell::new(None));
        let animation_window = window.clone();
        let animation_state = state.clone();
        let animation_handle = animation_frame.clone();
        *animation_frame.borrow_mut() = Some(Closure::wrap(Box::new(move |_time: f64| {
            if let Err(error) = animation_state.borrow_mut().render() {
                log_viewer_error(&error);
                return;
            }

            if let Some(callback) = animation_handle.borrow().as_ref() {
                if let Err(error) =
                    animation_window.request_animation_frame(callback.as_ref().unchecked_ref())
                {
                    log_viewer_error(&js_error(error));
                }
            }
        }) as Box<dyn FnMut(f64)>));

        if let Some(callback) = animation_frame.borrow().as_ref() {
            window
                .request_animation_frame(callback.as_ref().unchecked_ref())
                .map_err(js_error)?;
        }

        Ok(Self {
            _state: state,
            _resource_change: resource_change,
            _mouse_down: mouse_down,
            _mouse_move: mouse_move,
            _mouse_up: mouse_up,
            _mouse_leave: mouse_leave,
            _wheel: wheel,
            _resize: resize,
            _animation_frame: animation_frame,
        })
    }
}

#[cfg(target_arch = "wasm32")]
struct WebViewerState {
    window: Window,
    canvas: HtmlCanvasElement,
    axes_overlay: HtmlCanvasElement,
    axes_overlay_context: CanvasRenderingContext2d,
    resource_picker: HtmlSelectElement,
    status_line: HtmlElement,
    current_resource: String,
    runtime_scene: RuntimeSceneState,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: MeshRenderer,
    depth_target: DepthTarget,
    orbit: OrbitCameraController,
    drag: DragState,
}

#[cfg(target_arch = "wasm32")]
impl WebViewerState {
    fn begin_resource_load(&mut self, resource: &str) {
        self.resource_picker.set_disabled(true);
        self.resource_picker.set_value(resource);
        self.set_status(&format!("Loading {}...", friendly_resource_label(resource)));
    }

    fn finish_resource_load_failed(&mut self, resource: &str, error: &str) {
        self.resource_picker.set_disabled(false);
        self.resource_picker.set_value(&self.current_resource);
        self.set_status(&format!(
            "Failed to load {}: {}",
            friendly_resource_label(resource),
            error
        ));
    }

    fn apply_runtime_scene(&mut self, resource: String, runtime_scene: RuntimeSceneState) {
        self.runtime_scene = runtime_scene;
        self.current_resource = resource.clone();
        self.resource_picker.set_disabled(false);
        self.resource_picker.set_value(&resource);
        self.upload_runtime_scene(true);
    }

    fn resize_to_window(&mut self) -> Result<(), String> {
        let (width, height) =
            resize_canvases_to_window(&self.window, &self.canvas, &self.axes_overlay)?;
        if self.config.width == width && self.config.height == height {
            return Ok(());
        }

        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_target = DepthTarget::with_defaults(
            &self.device,
            ViewportSize::new(width, height),
            self.renderer.defaults(),
            "w web depth target",
        );
        self.renderer
            .resize(&self.queue, ViewportSize::new(width, height));
        self.draw_world_axes_overlay()?;

        Ok(())
    }

    fn upload_runtime_scene(&mut self, reset_camera: bool) {
        let render_scene = self.runtime_scene.compose_render_scene();
        self.renderer
            .upload_prepared_scene(&self.device, &self.queue, &render_scene);
        if reset_camera {
            let camera = fit_camera_to_render_scene(&render_scene);
            self.orbit = OrbitCameraController::from_camera(camera);
            self.renderer.set_camera(&self.queue, self.orbit.camera());
        }
        self.refresh_status();
    }

    fn frame_visible_scene(&mut self) {
        let render_scene = self.runtime_scene.compose_render_scene();
        let camera = fit_camera_to_render_scene(&render_scene);
        self.orbit = OrbitCameraController::from_camera(camera);
        self.renderer.set_camera(&self.queue, self.orbit.camera());
        self.refresh_status();
    }

    fn begin_drag(&mut self, x: f32, y: f32) {
        self.drag.active = true;
        self.drag.last_x = x;
        self.drag.last_y = y;
    }

    fn drag_to(&mut self, x: f32, y: f32) -> Result<(), String> {
        if !self.drag.active {
            return Ok(());
        }

        let dx = x - self.drag.last_x;
        let dy = y - self.drag.last_y;
        self.drag.last_x = x;
        self.drag.last_y = y;
        self.orbit.orbit_by_pixels(dx, dy);
        self.renderer.set_camera(&self.queue, self.orbit.camera());
        Ok(())
    }

    fn end_drag(&mut self) {
        self.drag.active = false;
    }

    fn zoom(&mut self, delta_y: f32) -> Result<(), String> {
        self.orbit.zoom_by_wheel(delta_y);
        self.renderer.set_camera(&self.queue, self.orbit.camera());
        Ok(())
    }

    fn render(&mut self) -> Result<(), String> {
        if self.config.width == 0 || self.config.height == 0 {
            return Ok(());
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.depth_target = DepthTarget::with_defaults(
                    &self.device,
                    ViewportSize::new(self.config.width, self.config.height),
                    self.renderer.defaults(),
                    "w web depth target",
                );
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("w web viewer hit a surface validation error".to_string());
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("w web frame encoder"),
            });
        self.renderer
            .render(&mut encoder, &view, self.depth_target.view());
        self.queue.submit([encoder.finish()]);
        frame.present();
        self.draw_world_axes_overlay()?;

        Ok(())
    }

    fn set_status(&self, message: &str) {
        self.status_line.set_text_content(Some(message));
    }

    fn refresh_status(&self) {
        self.set_status(&web_viewer_status_line(&self.runtime_scene));
    }

    fn draw_world_axes_overlay(&self) -> Result<(), String> {
        let device_pixel_ratio = self.window.device_pixel_ratio() as f32;
        let width = self.axes_overlay.width() as f32 / device_pixel_ratio.max(1.0);
        let height = self.axes_overlay.height() as f32 / device_pixel_ratio.max(1.0);
        if width <= 0.0 || height <= 0.0 {
            return Ok(());
        }

        self.axes_overlay_context
            .set_transform(
                f64::from(device_pixel_ratio),
                0.0,
                0.0,
                f64::from(device_pixel_ratio),
                0.0,
                0.0,
            )
            .map_err(js_error)?;
        self.axes_overlay_context
            .clear_rect(0.0, 0.0, f64::from(width), f64::from(height));

        let radius = 34.0_f32;
        let margin = 16.0_f32;
        let left_offset = -5.0_f32;
        let bottom_offset = 7.0_f32;
        let center = Vec2::new(
            margin + radius + left_offset,
            height - margin - radius + bottom_offset,
        );
        let axis_length = 24.0_f32;
        let origin_fill = "rgba(241, 244, 250, 0.82)";

        let mut axes = [
            overlay_axis(self.renderer.camera(), WORLD_RIGHT, "X", "#ee6352"),
            overlay_axis(self.renderer.camera(), WORLD_FORWARD, "Y", "#66d6a6"),
            overlay_axis(self.renderer.camera(), WORLD_UP, "Z", "#6ea8fe"),
        ];
        axes.sort_by(|left, right| right.depth.total_cmp(&left.depth));

        self.axes_overlay_context.set_fill_style_str(origin_fill);
        self.axes_overlay_context.begin_path();
        self.axes_overlay_context
            .arc(
                f64::from(center.x),
                f64::from(center.y),
                3.5,
                0.0,
                std::f64::consts::TAU,
            )
            .map_err(js_error)?;
        self.axes_overlay_context.fill();

        for axis in axes {
            draw_overlay_axis(&self.axes_overlay_context, center, axis_length, &axis)?;
        }

        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, Default)]
struct DragState {
    active: bool,
    last_x: f32,
    last_y: f32,
}

#[cfg(target_arch = "wasm32")]
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

#[cfg(target_arch = "wasm32")]
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

#[cfg(target_arch = "wasm32")]
fn typed_element<T>(document: &Document, id: &str) -> Result<T, String>
where
    T: JsCast,
{
    document
        .get_element_by_id(id)
        .ok_or_else(|| format!("w web viewer is missing the `#{id}` element"))?
        .dyn_into::<T>()
        .map_err(|_| format!("w web viewer found `#{id}` but it has the wrong element type"))
}

#[cfg(target_arch = "wasm32")]
fn populate_resource_picker(resource_picker: &HtmlSelectElement, resources: &[String]) {
    let options = resources
        .iter()
        .map(|resource| {
            format!(
                "<option value=\"{resource}\">{}</option>",
                friendly_resource_label(resource)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let element: &Element = resource_picker.unchecked_ref();
    element.set_inner_html(&options);
}

#[cfg(target_arch = "wasm32")]
fn friendly_resource_label(resource: &str) -> &str {
    match resource {
        "demo/mapped-pentagon-pair" => "mapped-pentagon-pair (per-instance color)",
        _ if resource.starts_with("demo/") => resource.trim_start_matches("demo/"),
        _ if resource.starts_with("ifc/") => resource.trim_start_matches("ifc/"),
        _ => resource,
    }
}

#[cfg(target_arch = "wasm32")]
fn initial_web_resource(window: &Window, resources: &[String]) -> String {
    window
        .location()
        .search()
        .ok()
        .and_then(|search| resource_from_search(&search))
        .filter(|resource| resources.iter().any(|candidate| candidate == resource))
        .unwrap_or_else(|| {
            resources
                .iter()
                .find(|resource| resource.as_str() == DEFAULT_WEB_RESOURCE)
                .cloned()
                .or_else(|| resources.first().cloned())
                .unwrap_or_else(|| DEFAULT_WEB_RESOURCE.to_string())
        })
}

#[cfg(target_arch = "wasm32")]
fn resource_from_search(search: &str) -> Option<String> {
    search.trim_start_matches('?').split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        if key != "resource" || value.is_empty() {
            return None;
        }

        decode_uri_component(value).ok()?.as_string()
    })
}

#[cfg(target_arch = "wasm32")]
fn resize_canvas_to_window(
    window: &Window,
    canvas: &HtmlCanvasElement,
) -> Result<(u32, u32), String> {
    let device_pixel_ratio = window.device_pixel_ratio();
    let client_width = canvas.client_width().max(1) as f64;
    let client_height = canvas.client_height().max(1) as f64;
    let fallback_width = window
        .inner_width()
        .map_err(js_error)?
        .as_f64()
        .unwrap_or(1280.0);
    let fallback_height = window
        .inner_height()
        .map_err(js_error)?
        .as_f64()
        .unwrap_or(720.0);
    let width = if client_width > 1.0 {
        client_width
    } else {
        fallback_width
    };
    let height = if client_height > 1.0 {
        client_height
    } else {
        fallback_height.max(240.0)
    };
    let surface_width = (width * device_pixel_ratio).round().max(1.0) as u32;
    let surface_height = (height * device_pixel_ratio).round().max(1.0) as u32;

    canvas.set_width(surface_width);
    canvas.set_height(surface_height);

    Ok((surface_width, surface_height))
}

#[cfg(target_arch = "wasm32")]
fn resize_canvases_to_window(
    window: &Window,
    canvas: &HtmlCanvasElement,
    axes_overlay: &HtmlCanvasElement,
) -> Result<(u32, u32), String> {
    let (surface_width, surface_height) = resize_canvas_to_window(window, canvas)?;
    axes_overlay.set_width(surface_width);
    axes_overlay.set_height(surface_height);
    Ok((surface_width, surface_height))
}

#[cfg(target_arch = "wasm32")]
fn axes_overlay_context(canvas: &HtmlCanvasElement) -> Result<CanvasRenderingContext2d, String> {
    canvas
        .get_context("2d")
        .map_err(js_error)?
        .ok_or("w web viewer could not create the axes overlay context".to_string())?
        .dyn_into::<CanvasRenderingContext2d>()
        .map_err(|_| {
            "w web viewer created the axes overlay with the wrong context type".to_string()
        })
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
struct OverlayAxis {
    label: &'static str,
    color: &'static str,
    direction: Vec2,
    depth: f32,
}

#[cfg(target_arch = "wasm32")]
fn overlay_axis(
    camera: Camera,
    axis: DVec3,
    label: &'static str,
    color: &'static str,
) -> OverlayAxis {
    let camera_space = camera.view_from_world().transform_vector3(axis);

    OverlayAxis {
        label,
        color,
        direction: Vec2::new(camera_space.x as f32, -(camera_space.y as f32)),
        depth: camera_space.z as f32,
    }
}

#[cfg(target_arch = "wasm32")]
fn draw_overlay_axis(
    context: &CanvasRenderingContext2d,
    center: Vec2,
    axis_length: f32,
    axis: &OverlayAxis,
) -> Result<(), String> {
    let vector = axis.direction * axis_length;
    let tip = center + vector;
    let head_direction = if axis.direction.length_squared() > 1.0e-6 {
        axis.direction.normalize()
    } else {
        Vec2::new(0.0, -1.0)
    };
    let arrow_head_length = 8.0;
    let arrow_head_angle = 0.55;
    let left = rotate_vec2(head_direction, arrow_head_angle) * arrow_head_length;
    let right = rotate_vec2(head_direction, -arrow_head_angle) * arrow_head_length;

    context.set_stroke_style_str(axis.color);
    context.set_fill_style_str(axis.color);
    context.set_line_width(2.25);

    context.begin_path();
    context.move_to(f64::from(center.x), f64::from(center.y));
    context.line_to(f64::from(tip.x), f64::from(tip.y));
    context.stroke();

    context.begin_path();
    context.move_to(f64::from(tip.x), f64::from(tip.y));
    context.line_to(f64::from((tip - left).x), f64::from((tip - left).y));
    context.move_to(f64::from(tip.x), f64::from(tip.y));
    context.line_to(f64::from((tip - right).x), f64::from((tip - right).y));
    context.stroke();

    let label_offset = if axis.direction.length_squared() > 1.0e-6 {
        axis.direction.normalize() * 12.0
    } else {
        Vec2::new(0.0, -12.0)
    };
    let label_position = tip + label_offset;
    context.set_font("12px Inter, system-ui, sans-serif");
    context.set_text_align("center");
    context.set_text_baseline("middle");
    context
        .fill_text(
            axis.label,
            f64::from(label_position.x),
            f64::from(label_position.y),
        )
        .map_err(js_error)?;

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn rotate_vec2(vector: Vec2, radians: f32) -> Vec2 {
    let (sin, cos) = radians.sin_cos();
    Vec2::new(
        (vector.x * cos) - (vector.y * sin),
        (vector.x * sin) + (vector.y * cos),
    )
}

#[cfg(target_arch = "wasm32")]
fn log_viewer_error(message: &str) {
    web_sys::console::error_1(&JsValue::from_str(message));
}

#[cfg(target_arch = "wasm32")]
fn js_error(error: JsValue) -> String {
    format!("{error:?}")
}

#[cfg(target_arch = "wasm32")]
fn with_web_viewer_state<T>(
    f: impl FnOnce(&WebViewerState) -> Result<T, String>,
) -> Result<T, JsValue> {
    WEB_VIEWER_APP.with(|slot| {
        let borrow = slot.borrow();
        let app = borrow
            .as_ref()
            .ok_or_else(|| JsValue::from_str("w web viewer is not initialized"))?;
        let state = app._state.borrow();
        f(&state).map_err(|error| JsValue::from_str(&error))
    })
}

#[cfg(target_arch = "wasm32")]
fn with_web_viewer_state_mut<T>(
    f: impl FnOnce(&mut WebViewerState) -> Result<T, String>,
) -> Result<T, JsValue> {
    WEB_VIEWER_APP.with(|slot| {
        let borrow = slot.borrow();
        let app = borrow
            .as_ref()
            .ok_or_else(|| JsValue::from_str("w web viewer is not initialized"))?;
        let mut state = app._state.borrow_mut();
        f(&mut state).map_err(|error| JsValue::from_str(&error))
    })
}

#[cfg(target_arch = "wasm32")]
fn semantic_ids_from_array(ids: &Array) -> Result<Vec<SemanticElementId>, String> {
    ids.iter()
        .enumerate()
        .map(|(index, value)| {
            let Some(id) = value.as_string() else {
                return Err(format!(
                    "w viewer expected a string semantic id at index {index}"
                ));
            };
            Ok(SemanticElementId::new(id))
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn semantic_ids_to_array<'a>(ids: impl IntoIterator<Item = &'a SemanticElementId>) -> Array {
    let array = Array::new();
    for id in ids {
        array.push(&JsValue::from_str(id.as_str()));
    }
    array
}

#[cfg(target_arch = "wasm32")]
fn web_viewer_status_line(runtime_scene: &RuntimeSceneState) -> String {
    let render_scene: PreparedRenderScene = runtime_scene.compose_render_scene();
    let visible_elements = runtime_scene.visible_element_ids().len();
    let total_elements = runtime_scene.package().elements.len();
    let selected_elements = runtime_scene.selected_element_ids().len();
    format!(
        "{}: {} triangles, {} draws, {visible_elements}/{total_elements} elements visible, {selected_elements} selected",
        runtime_scene.primary_label(),
        render_scene.triangle_count(),
        render_scene.draw_count(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_summary_mentions_demo_mesh() {
        let summary = demo_summary_string();

        assert!(summary.contains("Demo"));
    }
}
