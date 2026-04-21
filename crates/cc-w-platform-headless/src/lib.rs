mod snapshot;

pub use snapshot::{
    SnapshotComparison, SnapshotError, SnapshotOptions, SnapshotPaths,
    assert_rendered_image_snapshot, assert_snapshot,
};

use cc_w_backend::{
    DEFAULT_DEMO_RESOURCE, GeometryBackend, GeometryBackendError, ResourceError,
    available_demo_resources,
};
use cc_w_render::{
    Camera, HeadlessRenderError, NullRenderBackend, RenderedImage, ViewportSize,
    fit_camera_to_render_scene, render_prepared_scene_offscreen,
};
use cc_w_runtime::{
    DemoAsset, Engine, GeometryPackageSource, GeometryPackageSourceError, RuntimeError,
};
use cc_w_types::{PreparedGeometryPackage, WORLD_UP};
use cc_w_velr::{
    VelrIfcModel, available_ifc_body_resources, default_ifc_artifacts_root, ifc_body_resource_name,
    parse_ifc_body_resource,
};
use glam::DVec3;
use std::{
    fs::{self, File},
    io::BufWriter,
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const DEFAULT_VIEWPORT: ViewportSize = ViewportSize::new(512, 512);

const VISUAL_SUITE_VIEWPORT: ViewportSize = ViewportSize::new(512, 512);

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

#[derive(Clone, Debug, PartialEq)]
pub struct RenderRequest {
    pub resource: String,
    pub output: PathBuf,
    pub viewport: ViewportSize,
    pub camera: CameraRequest,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraRequest {
    pub eye: Option<DVec3>,
    pub target: DVec3,
    pub vertical_fov_degrees: f64,
}

impl Default for CameraRequest {
    fn default() -> Self {
        Self {
            eye: None,
            target: DVec3::ZERO,
            vertical_fov_degrees: 45.0,
        }
    }
}

impl CameraRequest {
    pub fn resolve_for_asset(&self, asset: &DemoAsset) -> Camera {
        let mut camera = fit_camera_to_render_scene(&asset.render_scene);
        camera.target = self.target;
        camera.vertical_fov_degrees = self.vertical_fov_degrees;
        if let Some(eye) = self.eye {
            camera.eye = eye;
        }

        let distance = camera.eye.distance(camera.target).max(0.1);
        camera.near_plane = (distance * 0.01).clamp(0.01, 0.5);
        camera.far_plane = camera.far_plane.max(distance * 16.0);
        camera
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Render(RenderRequest),
    VisualSuite { output_dir: PathBuf },
    ListResources,
    Help,
}

#[derive(Clone, Copy, Debug)]
enum VisualCameraMode {
    Fit,
    TopDown,
}

#[derive(Clone, Copy, Debug)]
struct VisualSuiteEntry {
    title: &'static str,
    resource: &'static str,
    filename: &'static str,
    camera: VisualCameraMode,
}

#[derive(Clone, Copy, Debug)]
struct VisualIndexEntry {
    title: &'static str,
    resource: &'static str,
    filename: &'static str,
}

const VISUAL_SUITE_ENTRIES: &[VisualSuiteEntry] = &[
    VisualSuiteEntry {
        title: "Triangle",
        resource: "demo/triangle",
        filename: "triangle.png",
        camera: VisualCameraMode::TopDown,
    },
    VisualSuiteEntry {
        title: "Polygon With Hole",
        resource: "demo/polygon-with-hole",
        filename: "polygon-with-hole.png",
        camera: VisualCameraMode::TopDown,
    },
    VisualSuiteEntry {
        title: "Concave Polygon",
        resource: "demo/concave-polygon",
        filename: "concave-polygon.png",
        camera: VisualCameraMode::TopDown,
    },
    VisualSuiteEntry {
        title: "Mapped Pentagon Pair (Per-Instance Color)",
        resource: "demo/mapped-pentagon-pair",
        filename: "mapped-pentagon-pair.png",
        camera: VisualCameraMode::TopDown,
    },
    VisualSuiteEntry {
        title: "Extruded Profile",
        resource: "demo/extruded-profile",
        filename: "extruded-profile.png",
        camera: VisualCameraMode::Fit,
    },
    VisualSuiteEntry {
        title: "Arc Extruded Profile",
        resource: "demo/arc-extruded-profile",
        filename: "arc-extruded-profile.png",
        camera: VisualCameraMode::Fit,
    },
    VisualSuiteEntry {
        title: "Revolved Solid",
        resource: "demo/revolved-solid",
        filename: "revolved-solid.png",
        camera: VisualCameraMode::Fit,
    },
    VisualSuiteEntry {
        title: "Circular Profile Sweep",
        resource: "demo/circular-profile-sweep",
        filename: "circular-profile-sweep.png",
        camera: VisualCameraMode::Fit,
    },
    VisualSuiteEntry {
        title: "Curved Circular Profile Sweep",
        resource: "demo/curved-circular-profile-sweep",
        filename: "curved-circular-profile-sweep.png",
        camera: VisualCameraMode::Fit,
    },
];

#[cfg(test)]
fn default_visual_e2e_output_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/visual-e2e")
}

#[derive(Debug, Error)]
pub enum HeadlessCliError {
    #[error("{0}")]
    Usage(String),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Render(#[from] HeadlessRenderError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Png(#[from] png::EncodingError),
}

impl HeadlessCliError {
    pub fn is_usage(&self) -> bool {
        matches!(self, Self::Usage(_))
    }
}

pub fn usage() -> String {
    format!(
        "\
Usage:
  cargo run -p cc-w-platform-headless -- --output /tmp/w.png [--resource {default_resource}] [--camera x,y,z] [--look-at x,y,z] [--size WIDTHxHEIGHT] [--fov degrees]
  cargo run -p cc-w-platform-headless -- --visual-suite /tmp/w-visual-e2e
  cargo run -p cc-w-platform-headless -- --list-resources

Notes:
  If --camera is omitted, w fits the camera to the selected mesh.
  --camera picks the eye position and --look-at picks the target.

Resources:
  {resources}
",
        default_resource = DEFAULT_DEMO_RESOURCE,
        resources = available_local_resources().join("\n  "),
    )
}

pub fn parse_args<I, S>(args: I) -> Result<Command, HeadlessCliError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|value| value.as_ref().to_string())
        .collect::<Vec<_>>();
    if args.is_empty() {
        return Ok(Command::Help);
    }

    let mut resource = DEFAULT_DEMO_RESOURCE.to_string();
    let mut output = None;
    let mut viewport = DEFAULT_VIEWPORT;
    let mut camera = CameraRequest::default();
    let mut list_resources = false;
    let mut visual_suite_output = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--list-resources" => {
                list_resources = true;
                index += 1;
            }
            "-r" | "--resource" => {
                let value = option_value(&args, index, "--resource")?;
                resource = value.to_string();
                index += 2;
            }
            "-o" | "--output" => {
                let value = option_value(&args, index, "--output")?;
                output = Some(PathBuf::from(value));
                index += 2;
            }
            "--visual-suite" => {
                let value = option_value(&args, index, "--visual-suite")?;
                visual_suite_output = Some(PathBuf::from(value));
                index += 2;
            }
            "--camera" | "--eye" => {
                let value = option_value(&args, index, "--camera")?;
                camera.eye = Some(parse_vec3(value)?);
                index += 2;
            }
            "--look-at" => {
                let value = option_value(&args, index, "--look-at")?;
                camera.target = parse_vec3(value)?;
                index += 2;
            }
            "--size" => {
                let value = option_value(&args, index, "--size")?;
                viewport = parse_viewport_size(value)?;
                index += 2;
            }
            "--fov" => {
                let value = option_value(&args, index, "--fov")?;
                camera.vertical_fov_degrees = parse_f64(value, "--fov")?;
                if camera.vertical_fov_degrees <= 1.0 || camera.vertical_fov_degrees >= 179.0 {
                    return Err(HeadlessCliError::Usage(
                        "--fov must be between 1 and 179 degrees".into(),
                    ));
                }
                index += 2;
            }
            unknown => {
                return Err(HeadlessCliError::Usage(format!(
                    "unknown argument `{unknown}`"
                )));
            }
        }
    }

    if list_resources && visual_suite_output.is_some() {
        return Err(HeadlessCliError::Usage(
            "--list-resources cannot be combined with --visual-suite".into(),
        ));
    }

    if list_resources {
        return Ok(Command::ListResources);
    }

    if let Some(output_dir) = visual_suite_output {
        return Ok(Command::VisualSuite { output_dir });
    }

    let output = output.ok_or_else(|| {
        HeadlessCliError::Usage("missing required argument `--output <path>`".into())
    })?;

    Ok(Command::Render(RenderRequest {
        resource,
        output,
        viewport,
        camera,
    }))
}

pub fn run_from_env() -> Result<String, HeadlessCliError> {
    run_command(parse_args(std::env::args().skip(1))?)
}

pub fn run_command(command: Command) -> Result<String, HeadlessCliError> {
    match command {
        Command::Help => Ok(usage()),
        Command::ListResources => Ok(available_local_resources().join("\n")),
        Command::VisualSuite { output_dir } => render_visual_suite_to_dir(&output_dir),
        Command::Render(request) => render_to_disk(&request),
    }
}

pub fn render_to_image(request: &RenderRequest) -> Result<RenderedImage, HeadlessCliError> {
    let engine = demo_engine();
    let asset = engine.build_demo_asset_for(&request.resource)?;
    let camera = request.camera.resolve_for_asset(&asset);
    Ok(pollster::block_on(render_prepared_scene_offscreen(
        &asset.render_scene,
        request.viewport,
        camera,
    ))?)
}

pub fn render_to_disk(request: &RenderRequest) -> Result<String, HeadlessCliError> {
    let engine = demo_engine();
    let asset = engine.build_demo_asset_for(&request.resource)?;
    let camera = request.camera.resolve_for_asset(&asset);
    let image = pollster::block_on(render_prepared_scene_offscreen(
        &asset.render_scene,
        request.viewport,
        camera,
    ))?;
    write_png(&request.output, &image)?;
    Ok(format!(
        "rendered {} to {} at {}x{}",
        asset.summary_line(),
        request.output.display(),
        request.viewport.width,
        request.viewport.height,
    ))
}

pub fn render_visual_suite_to_dir(output_dir: &Path) -> Result<String, HeadlessCliError> {
    fs::create_dir_all(output_dir)?;

    let engine = demo_engine();
    let mut rendered_entries = Vec::with_capacity(VISUAL_SUITE_ENTRIES.len());

    for entry in VISUAL_SUITE_ENTRIES {
        let asset = engine.build_demo_asset_for(entry.resource)?;
        let camera = match entry.camera {
            VisualCameraMode::Fit => CameraRequest::default().resolve_for_asset(&asset),
            VisualCameraMode::TopDown => {
                top_down_camera_for_extent(asset.bounds.size().max_element() as f32)
            }
        };
        let image = pollster::block_on(render_prepared_scene_offscreen(
            &asset.render_scene,
            VISUAL_SUITE_VIEWPORT,
            camera,
        ))?;
        let output_path = output_dir.join(entry.filename);
        write_png(&output_path, &image)?;
        rendered_entries.push(VisualIndexEntry {
            title: entry.title,
            resource: entry.resource,
            filename: entry.filename,
        });
    }

    write_visual_suite_index(output_dir, &rendered_entries)?;

    Ok(format!(
        "rendered visual e2e suite ({} images + index.html) to {}",
        rendered_entries.len(),
        output_dir.display(),
    ))
}

fn demo_engine() -> Engine<LocalGeometryBackendBridge> {
    Engine::new(
        LocalGeometryBackendBridge::default(),
        NullRenderBackend::default(),
    )
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

fn option_value<'a>(
    args: &'a [String],
    index: usize,
    option: &str,
) -> Result<&'a str, HeadlessCliError> {
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| HeadlessCliError::Usage(format!("missing value for `{option}`")))
}

fn parse_vec3(value: &str) -> Result<DVec3, HeadlessCliError> {
    let parts = value.split(',').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(HeadlessCliError::Usage(format!(
            "expected x,y,z vector, got `{value}`"
        )));
    }

    Ok(DVec3::new(
        parse_f64(parts[0], "vector x")?,
        parse_f64(parts[1], "vector y")?,
        parse_f64(parts[2], "vector z")?,
    ))
}

fn parse_viewport_size(value: &str) -> Result<ViewportSize, HeadlessCliError> {
    let (width, height) = value
        .split_once('x')
        .or_else(|| value.split_once('X'))
        .ok_or_else(|| {
            HeadlessCliError::Usage(format!(
                "expected viewport size WIDTHxHEIGHT, got `{value}`"
            ))
        })?;
    let width = parse_u32(width, "width")?;
    let height = parse_u32(height, "height")?;

    if width == 0 || height == 0 {
        return Err(HeadlessCliError::Usage(
            "viewport dimensions must be greater than zero".into(),
        ));
    }

    Ok(ViewportSize::new(width, height))
}

fn parse_f64(value: &str, field: &str) -> Result<f64, HeadlessCliError> {
    value
        .parse::<f64>()
        .map_err(|_| HeadlessCliError::Usage(format!("failed to parse {field} from `{value}`")))
}

fn parse_u32(value: &str, field: &str) -> Result<u32, HeadlessCliError> {
    value
        .parse::<u32>()
        .map_err(|_| HeadlessCliError::Usage(format!("failed to parse {field} from `{value}`")))
}

fn top_down_camera_for_extent(extent: f32) -> Camera {
    let distance = f64::from((extent * 2.25).max(7.0));

    Camera {
        eye: DVec3::new(0.0, 0.0, distance),
        target: DVec3::ZERO,
        up: WORLD_UP,
        vertical_fov_degrees: 28.0,
        near_plane: 0.1,
        far_plane: distance * 4.0,
    }
}

fn write_visual_suite_index(
    output_dir: &Path,
    rendered_entries: &[VisualIndexEntry],
) -> Result<(), HeadlessCliError> {
    let mut body = String::new();

    for entry in rendered_entries {
        body.push_str(&format!(
            "<figure><img src=\"{}\" alt=\"{}\"><figcaption><strong>{}</strong><br><code>{}</code></figcaption></figure>\n",
            entry.filename, entry.title, entry.title, entry.resource
        ));
    }

    let html = format!(
        "<!doctype html>
<html lang=\"en\">
<head>
  <meta charset=\"utf-8\">
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
  <title>w visual e2e</title>
  <style>
    body {{ font-family: ui-sans-serif, system-ui, sans-serif; margin: 24px; background: #101418; color: #f4f7fb; }}
    h1 {{ margin: 0 0 8px; font-size: 28px; }}
    p {{ margin: 0 0 24px; color: #b8c4d0; }}
    main {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 16px; }}
    figure {{ margin: 0; background: #182029; border: 1px solid #2d3945; border-radius: 8px; overflow: hidden; }}
    img {{ display: block; width: 100%; height: auto; background: #0d1117; }}
    figcaption {{ padding: 12px; line-height: 1.4; }}
    code {{ color: #8bd5ff; }}
  </style>
</head>
<body>
  <h1>w Visual E2E</h1>
  <p>Human verification renders for the current generic primitive coverage.</p>
  <main>
{body}  </main>
</body>
</html>
"
    );

    fs::write(output_dir.join("index.html"), html)?;
    Ok(())
}

pub fn write_png(path: &Path, image: &RenderedImage) -> Result<(), HeadlessCliError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.rgba8)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn parse_render_arguments() {
        let command = parse_args([
            "--resource",
            "triangle",
            "--camera",
            "0,0,7",
            "--look-at",
            "0,0,0",
            "--size",
            "640x480",
            "--fov",
            "35",
            "--output",
            "/tmp/triangle.png",
        ])
        .expect("command");

        let Command::Render(request) = command else {
            panic!("expected render command");
        };

        assert_eq!(request.resource, "triangle");
        assert_eq!(request.viewport, ViewportSize::new(640, 480));
        assert_eq!(request.camera.eye, Some(DVec3::new(0.0, 0.0, 7.0)));
        assert_eq!(request.camera.target, DVec3::ZERO);
        assert_eq!(request.camera.vertical_fov_degrees, 35.0);
        assert_eq!(request.output, PathBuf::from("/tmp/triangle.png"));
    }

    #[test]
    fn parse_list_resources_argument() {
        let command = parse_args(["--list-resources"]).expect("command");

        assert_eq!(command, Command::ListResources);
    }

    #[test]
    fn parse_visual_suite_argument() {
        let command = parse_args(["--visual-suite", "/tmp/w-visual-e2e"]).expect("command");

        assert_eq!(
            command,
            Command::VisualSuite {
                output_dir: PathBuf::from("/tmp/w-visual-e2e"),
            }
        );
    }

    #[test]
    fn camera_request_fits_mesh_when_eye_is_omitted() {
        let engine = demo_engine();
        let asset = engine.build_demo_asset_for("triangle").expect("asset");
        let camera = CameraRequest::default().resolve_for_asset(&asset);

        assert!(camera.eye.z > 0.0);
        assert_eq!(camera.target, DVec3::ZERO);
    }

    #[test]
    #[ignore = "human visual e2e smoke path; run on demand to dump PNG artifacts"]
    fn visual_suite_writes_artifacts_for_manual_review() {
        let output_dir = std::env::var("CC_W_VISUAL_E2E_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_visual_e2e_output_dir());

        let message = render_visual_suite_to_dir(&output_dir).expect("visual suite");

        assert!(message.contains("index.html"));
        assert!(output_dir.join("index.html").exists());
        assert!(output_dir.join("triangle.png").exists());
        assert!(output_dir.join("mapped-pentagon-pair.png").exists());
        assert!(output_dir.join("revolved-solid.png").exists());
    }

    #[test]
    fn write_png_persists_rgba_image() {
        let path = temp_path("write_png");
        let image = RenderedImage {
            width: 2,
            height: 1,
            rgba8: vec![255, 0, 0, 255, 0, 128, 255, 255],
        };

        write_png(&path, &image).expect("png");
        let bytes = fs::read(&path).expect("png bytes");

        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let _ = fs::remove_file(path);
    }

    fn temp_path(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();

        env::temp_dir().join(format!("{prefix}-{stamp}.png"))
    }
}
