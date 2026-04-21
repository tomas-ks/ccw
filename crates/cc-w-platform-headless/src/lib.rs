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
    IfcImportOptions, VelrIfcError, VelrIfcModel, available_ifc_body_resources,
    default_ifc_artifacts_root, ifc_body_resource_name, import_curated_fixture,
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

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

fn ensure_snapshot_resource_artifacts(resource: &str) -> Result<(), HeadlessCliError> {
    let Some(model_slug) = parse_ifc_body_resource(resource) else {
        return Ok(());
    };
    import_curated_fixture(model_slug, &IfcImportOptions::default())?;
    Ok(())
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
    pub target: Option<DVec3>,
    pub vertical_fov_degrees: f64,
}

impl Default for CameraRequest {
    fn default() -> Self {
        Self {
            eye: None,
            target: None,
            vertical_fov_degrees: 45.0,
        }
    }
}

impl CameraRequest {
    pub fn resolve_for_asset(&self, asset: &DemoAsset) -> Camera {
        let mut camera = fit_camera_to_render_scene(&asset.render_scene);
        if let Some(target) = self.target {
            camera.target = target;
        }
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
    SnapshotSuite { accepted_case: Option<String> },
    InvalidateSnapshot { case_id: String },
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

#[derive(Clone, Copy, Debug)]
struct SnapshotSuiteEntry {
    id: &'static str,
    resource: &'static str,
    filename: &'static str,
    viewport: ViewportSize,
    camera: CameraRequest,
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

const SNAPSHOT_SUITE_ENTRIES: &[SnapshotSuiteEntry] = &[
    SnapshotSuiteEntry {
        id: "demo-triangle",
        resource: "demo/triangle",
        filename: "demo-triangle-160x120.png",
        viewport: ViewportSize::new(160, 120),
        camera: CameraRequest {
            eye: Some(DVec3::new(0.0, 0.0, 7.0)),
            target: Some(DVec3::ZERO),
            vertical_fov_degrees: 45.0,
        },
    },
    SnapshotSuiteEntry {
        id: "demo-tilted-quad",
        resource: "demo/tilted-quad",
        filename: "demo-tilted-quad-160x120.png",
        viewport: ViewportSize::new(160, 120),
        camera: CameraRequest {
            eye: Some(DVec3::new(0.8, 0.5, 7.0)),
            target: Some(DVec3::ZERO),
            vertical_fov_degrees: 45.0,
        },
    },
    SnapshotSuiteEntry {
        id: "ifc-building-architecture",
        resource: "ifc/building-architecture",
        filename: "ifc-building-architecture-256x256.png",
        viewport: ViewportSize::new(256, 256),
        camera: CameraRequest {
            eye: None,
            target: None,
            vertical_fov_degrees: 45.0,
        },
    },
    SnapshotSuiteEntry {
        id: "ifc-building-hvac",
        resource: "ifc/building-hvac",
        filename: "ifc-building-hvac-256x256.png",
        viewport: ViewportSize::new(256, 256),
        camera: CameraRequest {
            eye: None,
            target: None,
            vertical_fov_degrees: 45.0,
        },
    },
    SnapshotSuiteEntry {
        id: "ifc-building-landscaping",
        resource: "ifc/building-landscaping",
        filename: "ifc-building-landscaping-256x256.png",
        viewport: ViewportSize::new(256, 256),
        camera: CameraRequest {
            eye: None,
            target: None,
            vertical_fov_degrees: 45.0,
        },
    },
    SnapshotSuiteEntry {
        id: "ifc-building-structural",
        resource: "ifc/building-structural",
        filename: "ifc-building-structural-256x256.png",
        viewport: ViewportSize::new(256, 256),
        camera: CameraRequest {
            eye: None,
            target: None,
            vertical_fov_degrees: 45.0,
        },
    },
];

fn available_snapshot_case_ids() -> Vec<&'static str> {
    SNAPSHOT_SUITE_ENTRIES
        .iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>()
}

fn find_snapshot_case(case_id: &str) -> Option<&'static SnapshotSuiteEntry> {
    SNAPSHOT_SUITE_ENTRIES
        .iter()
        .find(|entry| entry.id == case_id)
}

#[cfg(test)]
fn default_visual_e2e_output_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/visual-e2e")
}

#[derive(Debug, Error)]
pub enum HeadlessCliError {
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    SnapshotSuiteFailed(String),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Render(#[from] HeadlessRenderError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Png(#[from] png::EncodingError),
    #[error(transparent)]
    Ifc(#[from] VelrIfcError),
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
  cargo run -p cc-w-platform-headless -- --snapshot-suite [--accept-snapshot CASE_ID]
  cargo run -p cc-w-platform-headless -- --invalidate-snapshot CASE_ID
  cargo run -p cc-w-platform-headless -- --list-resources

Notes:
  If --camera is omitted, w fits the camera to the selected mesh.
  --camera picks the eye position and --look-at picks the target.
  Snapshot acceptance is one case at a time. Known CASE_ID values:
  {snapshot_cases}

Resources:
  {resources}
",
        default_resource = DEFAULT_DEMO_RESOURCE,
        snapshot_cases = available_snapshot_case_ids().join("\n  "),
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
    let mut snapshot_suite = false;
    let mut accepted_snapshot_case = None;
    let mut invalidated_snapshot_case = None;

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
            "--snapshot-suite" => {
                snapshot_suite = true;
                index += 1;
            }
            "--accept-snapshot" => {
                let value = option_value(&args, index, "--accept-snapshot")?;
                if accepted_snapshot_case.is_some() {
                    return Err(HeadlessCliError::Usage(
                        "--accept-snapshot can only be provided once".into(),
                    ));
                }
                accepted_snapshot_case = Some(value.to_string());
                index += 2;
            }
            "--invalidate-snapshot" => {
                let value = option_value(&args, index, "--invalidate-snapshot")?;
                if invalidated_snapshot_case.is_some() {
                    return Err(HeadlessCliError::Usage(
                        "--invalidate-snapshot can only be provided once".into(),
                    ));
                }
                invalidated_snapshot_case = Some(value.to_string());
                index += 2;
            }
            "--camera" | "--eye" => {
                let value = option_value(&args, index, "--camera")?;
                camera.eye = Some(parse_vec3(value)?);
                index += 2;
            }
            "--look-at" => {
                let value = option_value(&args, index, "--look-at")?;
                camera.target = Some(parse_vec3(value)?);
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

    if list_resources && snapshot_suite {
        return Err(HeadlessCliError::Usage(
            "--list-resources cannot be combined with --snapshot-suite".into(),
        ));
    }

    if visual_suite_output.is_some() && snapshot_suite {
        return Err(HeadlessCliError::Usage(
            "--visual-suite cannot be combined with --snapshot-suite".into(),
        ));
    }

    if output.is_some() && snapshot_suite {
        return Err(HeadlessCliError::Usage(
            "--output cannot be combined with --snapshot-suite".into(),
        ));
    }

    if accepted_snapshot_case.is_some() && !snapshot_suite {
        return Err(HeadlessCliError::Usage(
            "--accept-snapshot requires --snapshot-suite".into(),
        ));
    }

    if invalidated_snapshot_case.is_some()
        && (snapshot_suite
            || list_resources
            || visual_suite_output.is_some()
            || output.is_some()
            || accepted_snapshot_case.is_some())
    {
        return Err(HeadlessCliError::Usage(
            "--invalidate-snapshot cannot be combined with other commands".into(),
        ));
    }

    if list_resources {
        return Ok(Command::ListResources);
    }

    if let Some(case_id) = invalidated_snapshot_case {
        return Ok(Command::InvalidateSnapshot { case_id });
    }

    if snapshot_suite {
        return Ok(Command::SnapshotSuite {
            accepted_case: accepted_snapshot_case,
        });
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
        Command::SnapshotSuite { accepted_case } => render_snapshot_suite(accepted_case.as_deref()),
        Command::InvalidateSnapshot { case_id } => invalidate_snapshot_case(&case_id),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SnapshotReviewStatus {
    Validated,
    Accepted,
    MissingGolden,
    PixelMismatch,
    DimensionMismatch,
    RenderFailed,
    CompareFailed,
}

impl SnapshotReviewStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Validated => "Validated",
            Self::Accepted => "Accepted",
            Self::MissingGolden => "Missing Golden",
            Self::PixelMismatch => "Pixel Mismatch",
            Self::DimensionMismatch => "Dimension Mismatch",
            Self::RenderFailed => "Render Failed",
            Self::CompareFailed => "Compare Failed",
        }
    }

    fn needs_attention(self) -> bool {
        !matches!(self, Self::Validated | Self::Accepted)
    }
}

#[derive(Clone, Debug)]
struct SnapshotReviewCase {
    case_id: String,
    resource: String,
    status: SnapshotReviewStatus,
    detail: Option<String>,
    accepted_image: Option<String>,
    current_image: Option<String>,
    diff_image: Option<String>,
}

#[derive(Clone, Debug)]
struct SnapshotReviewReport {
    cases: Vec<SnapshotReviewCase>,
    index_path: PathBuf,
}

pub fn render_snapshot_suite(accepted_case: Option<&str>) -> Result<String, HeadlessCliError> {
    let artifact_dir = workspace_root().join("target").join("snapshot-artifacts");
    let review_dir = default_snapshot_review_output_dir();
    let golden_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden");
    let known_cases = available_snapshot_case_ids();

    if let Some(case_id) = accepted_case {
        if !known_cases.iter().any(|candidate| candidate == &case_id) {
            return Err(HeadlessCliError::Usage(format!(
                "unknown snapshot case `{case_id}`; known cases: {}",
                known_cases.join(", ")
            )));
        }
    }

    let report =
        build_snapshot_review_report(&artifact_dir, &review_dir, &golden_dir, accepted_case)?;
    let failing_cases = report
        .cases
        .iter()
        .filter(|case| case.status.needs_attention())
        .count();

    if failing_cases > 0 {
        return Err(HeadlessCliError::SnapshotSuiteFailed(format!(
            "headless snapshot suite found {failing_cases} case(s) that need review\nreview report: {}",
            report.index_path.display()
        )));
    }

    Ok(match accepted_case {
        Some(case_id) => format!(
            "accepted snapshot case `{case_id}` and validated {} cases\nreview report: {}",
            report.cases.len(),
            report.index_path.display()
        ),
        None => format!(
            "validated headless snapshot suite ({} cases)\nreview report: {}",
            report.cases.len(),
            report.index_path.display()
        ),
    })
}

pub fn invalidate_snapshot_case(case_id: &str) -> Result<String, HeadlessCliError> {
    let entry = find_snapshot_case(case_id).ok_or_else(|| {
        HeadlessCliError::Usage(format!(
            "unknown snapshot case `{case_id}`; known cases: {}",
            available_snapshot_case_ids().join(", ")
        ))
    })?;
    let golden_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(entry.filename);

    match fs::remove_file(&golden_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(format!(
                "snapshot case `{case_id}` already has no accepted golden at {}",
                golden_path.display()
            ));
        }
        Err(error) => return Err(error.into()),
    }

    let artifact_paths = SnapshotPaths::with_artifact_dir(
        golden_path.clone(),
        workspace_root().join("target").join("snapshot-artifacts"),
    );
    let _ = fs::remove_file(&artifact_paths.actual);
    let _ = fs::remove_file(&artifact_paths.diff);

    Ok(format!(
        "invalidated snapshot case `{case_id}` by removing {}",
        golden_path.display()
    ))
}

fn build_snapshot_review_report(
    artifact_dir: &Path,
    review_dir: &Path,
    golden_dir: &Path,
    accepted_case: Option<&str>,
) -> Result<SnapshotReviewReport, HeadlessCliError> {
    if review_dir.exists() {
        fs::remove_dir_all(review_dir)?;
    }
    let review_assets_dir = review_dir.join("assets");
    fs::create_dir_all(&review_assets_dir)?;
    fs::create_dir_all(artifact_dir)?;

    let mut cases = Vec::with_capacity(SNAPSHOT_SUITE_ENTRIES.len());
    for entry in SNAPSHOT_SUITE_ENTRIES {
        ensure_snapshot_resource_artifacts(entry.resource)?;
        let request = RenderRequest {
            resource: entry.resource.into(),
            output: artifact_dir.join("unused.png"),
            viewport: entry.viewport,
            camera: entry.camera,
        };
        let paths = SnapshotPaths::with_artifact_dir(golden_dir.join(entry.filename), artifact_dir);
        let mut review_case = SnapshotReviewCase {
            case_id: entry.id.to_string(),
            resource: entry.resource.to_string(),
            status: SnapshotReviewStatus::RenderFailed,
            detail: None,
            accepted_image: None,
            current_image: None,
            diff_image: None,
        };

        match render_to_image(&request) {
            Ok(current) => {
                let current_name = format!("{}.current.png", entry.id);
                let current_path = review_assets_dir.join(&current_name);
                write_png(&current_path, &current)?;
                review_case.current_image = Some(format!("assets/{current_name}"));

                match assert_rendered_image_snapshot(
                    &current,
                    &paths,
                    SnapshotOptions {
                        update_expected: accepted_case == Some(entry.id),
                        ..SnapshotOptions::default()
                    },
                ) {
                    Ok(_) => {
                        review_case.status = if accepted_case == Some(entry.id) {
                            SnapshotReviewStatus::Accepted
                        } else {
                            SnapshotReviewStatus::Validated
                        };
                    }
                    Err(SnapshotError::Io(error))
                        if error.kind() == std::io::ErrorKind::NotFound =>
                    {
                        review_case.status = SnapshotReviewStatus::MissingGolden;
                        review_case.detail =
                            Some("No accepted golden exists for this case yet.".into());
                    }
                    Err(SnapshotError::PixelMismatch {
                        mismatched_pixels,
                        max_channel_delta,
                        ..
                    }) => {
                        review_case.status = SnapshotReviewStatus::PixelMismatch;
                        review_case.detail = Some(format!(
                            "{mismatched_pixels} pixels differ (max channel delta {max_channel_delta})."
                        ));
                    }
                    Err(SnapshotError::DimensionMismatch {
                        expected_width,
                        expected_height,
                        actual_width,
                        actual_height,
                        ..
                    }) => {
                        review_case.status = SnapshotReviewStatus::DimensionMismatch;
                        review_case.detail = Some(format!(
                            "expected {expected_width}x{expected_height}, got {actual_width}x{actual_height}."
                        ));
                    }
                    Err(error) => {
                        review_case.status = SnapshotReviewStatus::CompareFailed;
                        review_case.detail = Some(error.to_string());
                    }
                }
            }
            Err(error) => {
                review_case.status = SnapshotReviewStatus::RenderFailed;
                review_case.detail = Some(error.to_string());
            }
        }

        if paths.expected.exists() {
            let accepted_name = format!("{}.accepted.png", entry.id);
            fs::copy(&paths.expected, review_assets_dir.join(&accepted_name))?;
            review_case.accepted_image = Some(format!("assets/{accepted_name}"));
        }
        if paths.diff.exists() {
            let diff_name = format!("{}.diff.png", entry.id);
            fs::copy(&paths.diff, review_assets_dir.join(&diff_name))?;
            review_case.diff_image = Some(format!("assets/{diff_name}"));
        }

        cases.push(review_case);
    }

    let index_path = write_snapshot_review_index(review_dir, &cases)?;
    Ok(SnapshotReviewReport { cases, index_path })
}

fn default_snapshot_review_output_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/snapshot-review")
}

fn write_snapshot_review_index(
    review_dir: &Path,
    cases: &[SnapshotReviewCase],
) -> Result<PathBuf, HeadlessCliError> {
    let mut attention_cards = String::new();
    let mut validated_cards = String::new();

    for case in cases {
        let card = snapshot_review_card(case);
        if case.status.needs_attention() {
            attention_cards.push_str(&card);
        } else {
            validated_cards.push_str(&card);
        }
    }

    let attention_section = if attention_cards.is_empty() {
        "<p class=\"empty\">Everything validated cleanly in this run.</p>\n".to_string()
    } else {
        attention_cards
    };
    let validated_section = if validated_cards.is_empty() {
        "<p class=\"empty\">No accepted baselines yet.</p>\n".to_string()
    } else {
        validated_cards
    };

    let html = format!(
        "<!doctype html>
<html lang=\"en\">
<head>
  <meta charset=\"utf-8\">
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
  <title>w Snapshot Review</title>
  <style>
    body {{ font-family: ui-sans-serif, system-ui, sans-serif; margin: 24px; background: #101418; color: #f4f7fb; }}
    h1 {{ margin: 0 0 8px; font-size: 28px; }}
    h2 {{ margin: 32px 0 12px; font-size: 20px; }}
    p {{ margin: 0 0 16px; color: #b8c4d0; }}
    code, pre {{ font-family: ui-monospace, SFMono-Regular, SFMono-Regular, monospace; }}
    .empty {{ color: #8ea0b2; }}
    .case-list {{ display: grid; gap: 16px; }}
    .card {{ background: #182029; border: 1px solid #2d3945; border-radius: 8px; overflow: hidden; }}
    .card-header {{ padding: 14px 16px 8px; border-bottom: 1px solid #24303b; }}
    .case-title {{ display: flex; flex-wrap: wrap; align-items: center; gap: 10px; margin-bottom: 8px; }}
    .badge {{ display: inline-block; padding: 4px 8px; border-radius: 6px; font-size: 12px; font-weight: 600; }}
    .badge.ok {{ background: #1f5535; color: #d9ffea; }}
    .badge.warn {{ background: #6b4d13; color: #ffeec8; }}
    .badge.err {{ background: #6a2430; color: #ffd7dc; }}
    .resource {{ color: #8bd5ff; font-size: 14px; }}
    .detail {{ color: #d7dee5; margin: 0; }}
    .commands {{ padding: 0 16px 16px; display: grid; gap: 8px; }}
    .command-label {{ color: #8ea0b2; font-size: 13px; }}
    .command {{ margin: 0; padding: 10px 12px; background: #0f151c; border: 1px solid #273340; border-radius: 6px; overflow-x: auto; }}
    .images {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; padding: 0 16px 16px; }}
    figure {{ margin: 0; background: #0f151c; border: 1px solid #24303b; border-radius: 6px; overflow: hidden; }}
    img {{ display: block; width: 100%; height: auto; background: #0b1117; }}
    figcaption {{ padding: 10px 12px; line-height: 1.4; color: #d7dee5; }}
  </style>
</head>
<body>
  <h1>w Snapshot Review</h1>
  <p>Accepted baselines, changed renders, and missing-golden cases from the latest headless snapshot run.</p>
  <h2>Needs Review</h2>
  <div class=\"case-list\">
{attention_section}  </div>
  <h2>Validated</h2>
  <div class=\"case-list\">
{validated_section}  </div>
</body>
</html>
"
    );

    let index_path = review_dir.join("index.html");
    fs::write(&index_path, html)?;
    Ok(index_path)
}

fn snapshot_review_card(case: &SnapshotReviewCase) -> String {
    let badge_class = match case.status {
        SnapshotReviewStatus::Validated | SnapshotReviewStatus::Accepted => "ok",
        SnapshotReviewStatus::MissingGolden => "warn",
        SnapshotReviewStatus::PixelMismatch
        | SnapshotReviewStatus::DimensionMismatch
        | SnapshotReviewStatus::RenderFailed
        | SnapshotReviewStatus::CompareFailed => "err",
    };
    let detail = case.detail.as_deref().unwrap_or("No differences detected.");
    let mut image_columns = String::new();
    image_columns.push_str(&snapshot_review_figure(
        "Accepted",
        case.accepted_image.as_deref(),
        "No accepted golden yet.",
    ));
    image_columns.push_str(&snapshot_review_figure(
        "Current",
        case.current_image.as_deref(),
        "No current render was produced.",
    ));
    image_columns.push_str(&snapshot_review_figure(
        "Diff",
        case.diff_image.as_deref(),
        "No diff image for this case.",
    ));

    format!(
        "<article class=\"card\">
  <div class=\"card-header\">
    <div class=\"case-title\">
      <strong>{case_id}</strong>
      <span class=\"badge {badge_class}\">{status}</span>
    </div>
    <div class=\"resource\"><code>{resource}</code></div>
    <p class=\"detail\">{detail}</p>
  </div>
  <div class=\"commands\">
    <div>
      <div class=\"command-label\">Accept this case</div>
      <pre class=\"command\"><code>{accept_cmd}</code></pre>
    </div>
    <div>
      <div class=\"command-label\">Invalidate this case</div>
      <pre class=\"command\"><code>{invalidate_cmd}</code></pre>
    </div>
  </div>
  <div class=\"images\">
{image_columns}  </div>
</article>
",
        case_id = escape_html(&case.case_id),
        badge_class = badge_class,
        status = case.status.label(),
        resource = escape_html(&case.resource),
        detail = escape_html(detail),
        accept_cmd = escape_html(&format!(
            "just headless-accept-snapshot case=\"{}\"",
            case.case_id
        )),
        invalidate_cmd = escape_html(&format!(
            "just headless-invalidate-snapshot case=\"{}\"",
            case.case_id
        )),
        image_columns = image_columns,
    )
}

fn snapshot_review_figure(title: &str, image: Option<&str>, fallback: &str) -> String {
    match image {
        Some(path) => format!(
            "<figure><img src=\"{}\" alt=\"{}\"><figcaption>{}</figcaption></figure>\n",
            escape_html(path),
            escape_html(title),
            escape_html(title),
        ),
        None => format!(
            "<figure><figcaption><strong>{}</strong><br>{}</figcaption></figure>\n",
            escape_html(title),
            escape_html(fallback),
        ),
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
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
        assert_eq!(request.camera.target, Some(DVec3::ZERO));
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
    fn parse_snapshot_suite_argument() {
        let command = parse_args(["--snapshot-suite", "--accept-snapshot", "ifc-building-hvac"])
            .expect("command");

        assert_eq!(
            command,
            Command::SnapshotSuite {
                accepted_case: Some("ifc-building-hvac".into()),
            }
        );
    }

    #[test]
    fn parse_invalidate_snapshot_argument() {
        let command = parse_args(["--invalidate-snapshot", "ifc-building-hvac"]).expect("command");

        assert_eq!(
            command,
            Command::InvalidateSnapshot {
                case_id: "ifc-building-hvac".into(),
            }
        );
    }

    #[test]
    fn camera_request_fits_mesh_when_eye_is_omitted() {
        let engine = demo_engine();
        let asset = engine.build_demo_asset_for("triangle").expect("asset");
        let fitted = fit_camera_to_render_scene(&asset.render_scene);
        let camera = CameraRequest::default().resolve_for_asset(&asset);

        assert_eq!(camera.eye, fitted.eye);
        assert_eq!(camera.target, fitted.target);
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
