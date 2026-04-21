use crate::{HeadlessCliError, RenderRequest, render_to_image, write_png};
use cc_w_render::RenderedImage;
use std::{
    fs::File,
    io::{BufReader, Error as IoError},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotOptions {
    pub tolerance_per_channel: u8,
    pub update_expected: bool,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            tolerance_per_channel: 0,
            update_expected: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotComparison {
    pub mismatched_pixels: usize,
    pub max_channel_delta: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotPaths {
    pub expected: PathBuf,
    pub actual: PathBuf,
    pub diff: PathBuf,
}

impl SnapshotPaths {
    pub fn from_expected(expected: impl Into<PathBuf>) -> Self {
        let expected = expected.into();
        let artifact_dir = expected.parent().unwrap_or_else(|| Path::new("."));

        Self {
            actual: artifact_dir.join(artifact_name(&expected, "actual")),
            diff: artifact_dir.join(artifact_name(&expected, "diff")),
            expected,
        }
    }

    pub fn with_artifact_dir(
        expected: impl Into<PathBuf>,
        artifact_dir: impl Into<PathBuf>,
    ) -> Self {
        let expected = expected.into();
        let artifact_dir = artifact_dir.into();

        Self {
            actual: artifact_dir.join(artifact_name(&expected, "actual")),
            diff: artifact_dir.join(artifact_name(&expected, "diff")),
            expected,
        }
    }
}

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error(transparent)]
    Render(#[from] HeadlessCliError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Decode(#[from] png::DecodingError),
    #[error("expected RGBA8 PNG at `{path}`, got {color_type:?} {bit_depth:?}")]
    UnsupportedPng {
        path: PathBuf,
        color_type: png::ColorType,
        bit_depth: png::BitDepth,
    },
    #[error(
        "snapshot dimensions differ for `{expected_path}`: expected {expected_width}x{expected_height}, got {actual_width}x{actual_height}; wrote actual to `{actual_path}` and diff to `{diff_path}`"
    )]
    DimensionMismatch {
        expected_path: PathBuf,
        actual_path: PathBuf,
        diff_path: PathBuf,
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },
    #[error(
        "snapshot mismatch for `{expected_path}`: {mismatched_pixels} pixels differ (max channel delta {max_channel_delta}, tolerance {tolerance_per_channel}); wrote actual to `{actual_path}` and diff to `{diff_path}`"
    )]
    PixelMismatch {
        expected_path: PathBuf,
        actual_path: PathBuf,
        diff_path: PathBuf,
        tolerance_per_channel: u8,
        mismatched_pixels: usize,
        max_channel_delta: u8,
    },
}

pub fn assert_snapshot(
    request: &RenderRequest,
    paths: &SnapshotPaths,
    options: SnapshotOptions,
) -> Result<SnapshotComparison, SnapshotError> {
    let actual = render_to_image(request)?;
    assert_rendered_image_snapshot(&actual, paths, options)
}

pub fn assert_rendered_image_snapshot(
    actual: &RenderedImage,
    paths: &SnapshotPaths,
    options: SnapshotOptions,
) -> Result<SnapshotComparison, SnapshotError> {
    let expected = match read_png(&paths.expected) {
        Ok(expected) => expected,
        Err(SnapshotError::Io(error))
            if error.kind() == std::io::ErrorKind::NotFound && options.update_expected =>
        {
            write_png(&paths.expected, actual)?;
            clear_snapshot_artifacts(paths)?;
            return Ok(accepted_snapshot_result());
        }
        Err(error) => return Err(error),
    };

    if expected.width != actual.width || expected.height != actual.height {
        if options.update_expected {
            write_png(&paths.expected, actual)?;
            clear_snapshot_artifacts(paths)?;
            return Ok(accepted_snapshot_result());
        }
        write_png(&paths.actual, actual)?;
        write_png(&paths.diff, &dimension_mismatch_image(&expected, actual))?;

        return Err(SnapshotError::DimensionMismatch {
            expected_path: paths.expected.clone(),
            actual_path: paths.actual.clone(),
            diff_path: paths.diff.clone(),
            expected_width: expected.width,
            expected_height: expected.height,
            actual_width: actual.width,
            actual_height: actual.height,
        });
    }

    let comparison = compare_images(&expected, actual, options.tolerance_per_channel);
    if comparison.mismatched_pixels > 0 {
        if options.update_expected {
            write_png(&paths.expected, actual)?;
            clear_snapshot_artifacts(paths)?;
            return Ok(accepted_snapshot_result());
        }
        write_png(&paths.actual, actual)?;
        write_png(
            &paths.diff,
            &diff_image(&expected, actual, options.tolerance_per_channel),
        )?;

        return Err(SnapshotError::PixelMismatch {
            expected_path: paths.expected.clone(),
            actual_path: paths.actual.clone(),
            diff_path: paths.diff.clone(),
            tolerance_per_channel: options.tolerance_per_channel,
            mismatched_pixels: comparison.mismatched_pixels,
            max_channel_delta: comparison.max_channel_delta,
        });
    }

    clear_snapshot_artifacts(paths)?;
    Ok(comparison)
}

fn accepted_snapshot_result() -> SnapshotComparison {
    SnapshotComparison {
        mismatched_pixels: 0,
        max_channel_delta: 0,
    }
}

fn clear_snapshot_artifacts(paths: &SnapshotPaths) -> Result<(), SnapshotError> {
    remove_if_exists(&paths.actual)?;
    remove_if_exists(&paths.diff)?;
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<(), SnapshotError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn read_png(path: &Path) -> Result<RenderedImage, SnapshotError> {
    let decoder = png::Decoder::new(BufReader::new(File::open(path)?));
    let mut reader = decoder.read_info()?;
    let output_size = reader.output_buffer_size().ok_or_else(|| {
        IoError::other(format!(
            "decoded PNG at `{}` is too large to fit in memory",
            path.display()
        ))
    })?;
    let mut buffer = vec![0; output_size];
    let info = reader.next_frame(&mut buffer)?;

    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(SnapshotError::UnsupportedPng {
            path: path.to_path_buf(),
            color_type: info.color_type,
            bit_depth: info.bit_depth,
        });
    }

    buffer.truncate(info.buffer_size());
    Ok(RenderedImage {
        width: info.width,
        height: info.height,
        rgba8: buffer,
    })
}

fn compare_images(
    expected: &RenderedImage,
    actual: &RenderedImage,
    tolerance_per_channel: u8,
) -> SnapshotComparison {
    let mut mismatched_pixels = 0;
    let mut max_channel_delta = 0;

    for (expected_pixel, actual_pixel) in expected
        .rgba8
        .chunks_exact(4)
        .zip(actual.rgba8.chunks_exact(4))
    {
        let pixel_max_delta = expected_pixel
            .iter()
            .zip(actual_pixel)
            .map(|(expected_channel, actual_channel)| expected_channel.abs_diff(*actual_channel))
            .max()
            .unwrap_or(0);

        max_channel_delta = max_channel_delta.max(pixel_max_delta);
        if pixel_max_delta > tolerance_per_channel {
            mismatched_pixels += 1;
        }
    }

    SnapshotComparison {
        mismatched_pixels,
        max_channel_delta,
    }
}

fn diff_image(
    expected: &RenderedImage,
    actual: &RenderedImage,
    tolerance_per_channel: u8,
) -> RenderedImage {
    let mut rgba8 = Vec::with_capacity(actual.rgba8.len());

    for (expected_pixel, actual_pixel) in expected
        .rgba8
        .chunks_exact(4)
        .zip(actual.rgba8.chunks_exact(4))
    {
        let pixel_max_delta = expected_pixel
            .iter()
            .zip(actual_pixel)
            .map(|(expected_channel, actual_channel)| expected_channel.abs_diff(*actual_channel))
            .max()
            .unwrap_or(0);

        if pixel_max_delta > tolerance_per_channel {
            rgba8.extend_from_slice(&[255, pixel_max_delta, 0, 255]);
        } else {
            rgba8.extend_from_slice(&[
                expected_pixel[0] / 4,
                expected_pixel[1] / 4,
                expected_pixel[2] / 4,
                255,
            ]);
        }
    }

    RenderedImage {
        width: actual.width,
        height: actual.height,
        rgba8,
    }
}

fn dimension_mismatch_image(expected: &RenderedImage, actual: &RenderedImage) -> RenderedImage {
    let width = expected.width.max(actual.width);
    let height = expected.height.max(actual.height);
    let mut rgba8 = vec![0; (width * height * 4) as usize];

    for pixel in rgba8.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[255, 0, 255, 255]);
    }

    RenderedImage {
        width,
        height,
        rgba8,
    }
}

fn artifact_name(expected: &Path, suffix: &str) -> String {
    let stem = expected
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("snapshot");
    let extension = expected
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");

    format!("{stem}.{suffix}.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_png;
    use std::{
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn snapshot_paths_keep_expected_basename() {
        let paths =
            SnapshotPaths::with_artifact_dir("/tmp/demo-triangle.png", "/tmp/snapshot-artifacts");

        assert_eq!(
            paths.actual,
            PathBuf::from("/tmp/snapshot-artifacts/demo-triangle.actual.png")
        );
        assert_eq!(
            paths.diff,
            PathBuf::from("/tmp/snapshot-artifacts/demo-triangle.diff.png")
        );
    }

    #[test]
    fn compare_images_counts_pixel_mismatches() {
        let expected = solid_image([0, 0, 0, 255]);
        let actual = RenderedImage {
            width: 2,
            height: 1,
            rgba8: vec![0, 0, 0, 255, 12, 0, 0, 255],
        };

        let comparison = compare_images(&expected, &actual, 4);

        assert_eq!(comparison.mismatched_pixels, 1);
        assert_eq!(comparison.max_channel_delta, 12);
    }

    #[test]
    fn mismatched_snapshot_writes_actual_and_diff_artifacts() {
        let directory = temp_dir("snapshot-mismatch");
        let expected_path = directory.join("expected.png");
        let paths = SnapshotPaths {
            expected: expected_path.clone(),
            actual: directory.join("actual.png"),
            diff: directory.join("diff.png"),
        };
        let expected = solid_image([0, 0, 0, 255]);
        let actual = solid_image([255, 0, 0, 255]);

        write_png(&expected_path, &expected).expect("expected png");
        let error = assert_rendered_image_snapshot(&actual, &paths, SnapshotOptions::default())
            .expect_err("snapshot should fail");

        assert!(matches!(error, SnapshotError::PixelMismatch { .. }));
        assert!(paths.actual.exists());
        assert!(paths.diff.exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn accept_mode_writes_missing_expected_snapshot() {
        let directory = temp_dir("snapshot-accept-missing");
        let paths = SnapshotPaths {
            expected: directory.join("expected.png"),
            actual: directory.join("actual.png"),
            diff: directory.join("diff.png"),
        };
        let actual = solid_image([64, 128, 192, 255]);

        let comparison = assert_rendered_image_snapshot(
            &actual,
            &paths,
            SnapshotOptions {
                update_expected: true,
                ..SnapshotOptions::default()
            },
        )
        .expect("snapshot should be accepted");

        assert_eq!(comparison.mismatched_pixels, 0);
        assert!(paths.expected.exists());
        assert!(!paths.actual.exists());
        assert!(!paths.diff.exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn accept_mode_replaces_mismatched_expected_snapshot() {
        let directory = temp_dir("snapshot-accept-replace");
        let expected_path = directory.join("expected.png");
        let paths = SnapshotPaths {
            expected: expected_path.clone(),
            actual: directory.join("actual.png"),
            diff: directory.join("diff.png"),
        };
        let expected = solid_image([0, 0, 0, 255]);
        let actual = solid_image([255, 0, 0, 255]);

        write_png(&expected_path, &expected).expect("expected png");
        let comparison = assert_rendered_image_snapshot(
            &actual,
            &paths,
            SnapshotOptions {
                update_expected: true,
                ..SnapshotOptions::default()
            },
        )
        .expect("snapshot should be accepted");
        let written = read_png(&expected_path).expect("accepted png");

        assert_eq!(comparison.mismatched_pixels, 0);
        assert_eq!(written, actual);
        assert!(!paths.actual.exists());
        assert!(!paths.diff.exists());
        let _ = fs::remove_dir_all(directory);
    }

    fn solid_image(pixel: [u8; 4]) -> RenderedImage {
        let mut rgba8 = Vec::new();
        rgba8.extend_from_slice(&pixel);
        rgba8.extend_from_slice(&pixel);

        RenderedImage {
            width: 2,
            height: 1,
            rgba8,
        }
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let directory = env::temp_dir().join(format!("{prefix}-{stamp}"));
        fs::create_dir_all(&directory).expect("temp dir");
        directory
    }
}
