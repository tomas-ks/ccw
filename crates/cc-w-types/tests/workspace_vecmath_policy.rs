use std::{
    fs,
    path::{Path, PathBuf},
};

const DISALLOWED_VECTMATH_CRATES: &[&str] = &[
    "nalgebra",
    "cgmath",
    "ultraviolet",
    "euclid",
    "vek",
    "mint",
    "simba",
];

#[test]
fn workspace_uses_glam_as_the_only_vecmath_dependency() {
    for manifest in workspace_manifests() {
        let contents = fs::read_to_string(&manifest).expect("manifest");

        for crate_name in DISALLOWED_VECTMATH_CRATES {
            assert!(
                !manifest_declares_dependency(&contents, crate_name),
                "unexpected vecmath dependency `{crate_name}` in {}",
                manifest.display()
            );
        }
    }
}

#[test]
fn crates_share_workspace_glam_dependency() {
    let root = workspace_root();
    let root_manifest = fs::read_to_string(root.join("Cargo.toml")).expect("root Cargo.toml");

    assert!(
        root_manifest.contains("glam = { version = "),
        "workspace root should pin glam once in [workspace.dependencies]"
    );

    for manifest in crate_manifests() {
        let contents = fs::read_to_string(&manifest).expect("crate manifest");
        if !manifest_declares_dependency(&contents, "glam") {
            continue;
        }

        assert!(
            contents.contains("glam.workspace = true"),
            "crate manifest should use workspace glam dependency: {}",
            manifest.display()
        );
        assert!(
            !contents.contains("glam = {") && !contents.contains("glam = \""),
            "crate manifest should not pin a local glam version: {}",
            manifest.display()
        );
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn workspace_manifests() -> Vec<PathBuf> {
    let mut manifests = vec![workspace_root().join("Cargo.toml")];
    manifests.extend(crate_manifests());
    manifests
}

fn crate_manifests() -> Vec<PathBuf> {
    let mut manifests = fs::read_dir(workspace_root().join("crates"))
        .expect("crates dir")
        .map(|entry| entry.expect("crate dir").path().join("Cargo.toml"))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    manifests.sort();
    manifests
}

fn manifest_declares_dependency(manifest: &str, dependency: &str) -> bool {
    manifest.lines().any(|line| {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix(dependency) else {
            return false;
        };

        matches!(rest.chars().next(), Some(' ') | Some('=') | Some('.'))
    })
}
