use cc_w_platform_headless::{
    CameraRequest, RenderRequest, SnapshotOptions, SnapshotPaths, assert_snapshot,
};
use cc_w_render::ViewportSize;
use glam::DVec3;
use std::path::{Path, PathBuf};

#[test]
fn demo_triangle_matches_golden_snapshot() {
    run_snapshot_case(
        "demo/triangle",
        "demo-triangle-160x120.png",
        DVec3::new(0.0, 0.0, 7.0),
    );
}

#[test]
fn demo_tilted_quad_matches_golden_snapshot() {
    run_snapshot_case(
        "demo/tilted-quad",
        "demo-tilted-quad-160x120.png",
        DVec3::new(0.8, 0.5, 7.0),
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

fn run_snapshot_case(resource: &str, golden_name: &str, eye: DVec3) {
    if std::env::var_os("CC_W_RUN_SNAPSHOT_TESTS").is_none() {
        eprintln!("skipping golden snapshot test; set CC_W_RUN_SNAPSHOT_TESTS=1 to enable it");
        return;
    }

    let artifact_dir = workspace_root().join("target").join("snapshot-artifacts");
    let expected = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(golden_name);
    let request = RenderRequest {
        resource: resource.into(),
        output: artifact_dir.join("unused.png"),
        viewport: ViewportSize::new(160, 120),
        camera: CameraRequest {
            eye: Some(eye),
            target: DVec3::ZERO,
            vertical_fov_degrees: 45.0,
        },
    };
    let paths = SnapshotPaths::with_artifact_dir(expected, &artifact_dir);

    let comparison =
        assert_snapshot(&request, &paths, SnapshotOptions::default()).expect("golden snapshot");

    assert_eq!(comparison.mismatched_pixels, 0);
}
