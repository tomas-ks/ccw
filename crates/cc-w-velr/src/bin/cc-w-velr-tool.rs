use std::env;
use std::path::PathBuf;
use std::process;

use cc_w_velr::{
    IfcArtifactLayout, IfcImportOptions, IfcSchemaId, VelrIfcModel, clear_all_ifc_geometry_caches,
    clear_all_ifc_legacy_runtime_sidecars, clear_all_ifc_model_artifacts, clear_ifc_geometry_cache,
    clear_ifc_legacy_runtime_sidecars, clear_ifc_model_artifacts, curated_fixture_specs,
    default_ifc_artifacts_root, default_ifc_fixtures_root, default_velr_ifc_checkout,
    import_curated_fixture, import_ifc_file, refresh_ifc_runtime_sidecars,
    refresh_ifc_schema_runtime_sidecars, slugify_model_name, sync_curated_fixtures,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("w velr tool failed: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_usage();
        return Ok(());
    };

    match command.as_str() {
        "list-fixtures" => {
            for fixture in curated_fixture_specs() {
                println!("{} -> fixtures/ifc/{}", fixture.slug, fixture.file_name);
            }
        }
        "sync-fixtures" => {
            let mut velr_ifc_root = default_velr_ifc_checkout();
            let mut fixtures_root = default_ifc_fixtures_root();

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--velr-ifc-root" => {
                        velr_ifc_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    "--fixtures-root" => {
                        fixtures_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    _ => return Err(format!("unknown option for sync-fixtures: {arg}").into()),
                }
            }

            let results = sync_curated_fixtures(&velr_ifc_root, &fixtures_root)?;
            println!("synced {} curated IFC fixtures", results.len());
            for result in results {
                println!(
                    "- {} -> {} ({} bytes)",
                    result.slug,
                    result.destination.display(),
                    result.bytes
                );
            }
        }
        "import" => {
            let mut options = IfcImportOptions::default();
            let mut fixture = None;
            let mut ifc_path = None;
            let mut model = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--fixture" => fixture = Some(require_value(&mut args, &arg)?),
                    "--ifc" => ifc_path = Some(PathBuf::from(require_value(&mut args, &arg)?)),
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--artifacts-root" => {
                        options.artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    "--velr-ifc-root" => {
                        options.velr_ifc_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    "--debug" => options.release = false,
                    "--release" => options.release = true,
                    "--debug-artifacts" => options.debug_artifacts = true,
                    "--no-debug-artifacts" => options.debug_artifacts = false,
                    "--replace-existing" => options.replace_existing = true,
                    _ => return Err(format!("unknown option for import: {arg}").into()),
                }
            }

            let summary = if let Some(fixture_slug) = fixture {
                import_curated_fixture(&fixture_slug, &options)?
            } else if let Some(step_input) = ifc_path {
                let model_slug = model.unwrap_or_else(|| {
                    step_input
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(slugify_model_name)
                        .unwrap_or_else(|| "imported-ifc".to_string())
                });
                import_ifc_file(step_input, model_slug, &options)?
            } else {
                return Err("import requires either --fixture <slug> or --ifc <path>".into());
            };

            println!("imported IFC model `{}`", summary.model_slug);
            println!(
                "import_status: {}",
                if summary.reused_existing {
                    "reused-existing"
                } else {
                    "imported-fresh"
                }
            );
            println!("model_root: {}", summary.model_root.display());
            println!("database: {}", summary.database.display());
            println!("schema: {}", summary.schema.canonical_name());
            println!("import_timing: {}", summary.import_timing.display());
            println!("import_log: {}", summary.import_log.display());
        }
        "import-fixtures" => {
            let mut options = IfcImportOptions::default();

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--artifacts-root" => {
                        options.artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    "--velr-ifc-root" => {
                        options.velr_ifc_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    "--debug" => options.release = false,
                    "--release" => options.release = true,
                    "--debug-artifacts" => options.debug_artifacts = true,
                    "--no-debug-artifacts" => options.debug_artifacts = false,
                    "--replace-existing" => options.replace_existing = true,
                    _ => return Err(format!("unknown option for import-fixtures: {arg}").into()),
                }
            }

            let mut reused = 0_usize;
            let mut imported = 0_usize;

            for fixture in curated_fixture_specs() {
                let summary = import_curated_fixture(fixture.slug, &options)?;
                println!(
                    "- {} {}",
                    summary.model_slug,
                    if summary.reused_existing {
                        "reused-existing"
                    } else {
                        "imported-fresh"
                    }
                );
                if summary.reused_existing {
                    reused += 1;
                } else {
                    imported += 1;
                }
            }

            println!("fixture_imports_total: {}", curated_fixture_specs().len());
            println!("fixture_imports_reused: {reused}");
            println!("fixture_imports_fresh: {imported}");
        }
        "clear-artifacts" => {
            let mut artifacts_root = default_ifc_artifacts_root();
            let mut model = None;
            let mut clear_all = false;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--all" => clear_all = true,
                    "--artifacts-root" => {
                        artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    _ => return Err(format!("unknown option for clear-artifacts: {arg}").into()),
                }
            }

            if clear_all {
                let cleared = clear_all_ifc_model_artifacts(&artifacts_root)?;
                println!("cleared_artifact_models: {cleared}");
            } else {
                let model = model.ok_or("clear-artifacts requires --model <slug> or --all")?;
                let cleared = clear_ifc_model_artifacts(&artifacts_root, &model)?;
                println!("cleared_artifact_model: {model}");
                println!("cleared: {cleared}");
            }
        }
        "clear-geometry-cache" => {
            let mut artifacts_root = default_ifc_artifacts_root();
            let mut model = None;
            let mut clear_all = false;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--all" => clear_all = true,
                    "--artifacts-root" => {
                        artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    _ => {
                        return Err(
                            format!("unknown option for clear-geometry-cache: {arg}").into()
                        );
                    }
                }
            }

            if clear_all {
                let cleared = clear_all_ifc_geometry_caches(&artifacts_root)?;
                println!("cleared_geometry_caches: {cleared}");
            } else {
                let model = model.ok_or("clear-geometry-cache requires --model <slug> or --all")?;
                let cleared = clear_ifc_geometry_cache(&artifacts_root, &model)?;
                println!("cleared_geometry_cache_model: {model}");
                println!("cleared: {cleared}");
            }
        }
        "clear-legacy-runtime" => {
            let mut artifacts_root = default_ifc_artifacts_root();
            let mut model = None;
            let mut clear_all = false;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--all" => clear_all = true,
                    "--artifacts-root" => {
                        artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    _ => {
                        return Err(
                            format!("unknown option for clear-legacy-runtime: {arg}").into()
                        );
                    }
                }
            }

            if clear_all {
                let cleared = clear_all_ifc_legacy_runtime_sidecars(&artifacts_root)?;
                println!("cleared_legacy_runtime_dirs: {cleared}");
            } else {
                let model = model.ok_or("clear-legacy-runtime requires --model <slug> or --all")?;
                let cleared = clear_ifc_legacy_runtime_sidecars(&artifacts_root, &model)?;
                println!("cleared_legacy_runtime_model: {model}");
                println!("cleared: {cleared}");
            }
        }
        "refresh-runtime" => {
            let mut artifacts_root = default_ifc_artifacts_root();
            let mut velr_ifc_root = default_velr_ifc_checkout();
            let mut model = None;
            let mut schema = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--schema" => schema = Some(require_value(&mut args, &arg)?),
                    "--artifacts-root" => {
                        artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    "--velr-ifc-root" => {
                        velr_ifc_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    _ => return Err(format!("unknown option for refresh-runtime: {arg}").into()),
                }
            }

            let refreshed = match (model, schema) {
                (Some(model), None) => {
                    refresh_ifc_runtime_sidecars(&artifacts_root, &model, &velr_ifc_root)?
                }
                (None, Some(schema)) => refresh_ifc_schema_runtime_sidecars(
                    &artifacts_root,
                    IfcSchemaId::parse(&schema),
                    &velr_ifc_root,
                )?,
                (Some(_), Some(_)) => {
                    return Err(
                        "refresh-runtime accepts either --model <slug> or --schema <ifc-schema>, not both"
                            .into(),
                    )
                }
                (None, None) => {
                    return Err(
                        "refresh-runtime requires --model <slug> or --schema <ifc-schema>".into(),
                    )
                }
            };

            println!("schema: {}", refreshed.schema.canonical_name());
            println!("graphql_runtime_root: {}", refreshed.root.display());
            println!("runtime_graphql: {}", refreshed.runtime_graphql.display());
            println!("runtime_mapping: {}", refreshed.runtime_mapping.display());
            println!("runtime_manifest: {}", refreshed.runtime_manifest.display());
        }
        "query-projects" => {
            let mut artifacts_root = default_ifc_artifacts_root();
            let mut model = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--artifacts-root" => {
                        artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    _ => return Err(format!("unknown option for query-projects: {arg}").into()),
                }
            }

            let model = model.ok_or("query-projects requires --model <slug>")?;
            let handle = VelrIfcModel::open(IfcArtifactLayout::new(&artifacts_root, model))?;
            let projects = match handle.query_projects_graphql() {
                Ok(projects) => {
                    println!("query_source: graphql");
                    projects
                }
                Err(error) => {
                    eprintln!("w query-projects falling back to raw cypher: {error}");
                    println!("query_source: raw-cypher-fallback");
                    handle.query_projects_raw()?
                }
            };
            println!("projects: {}", projects.len());
            for project in projects {
                println!(
                    "- {} [{}] global_id={} name={} long_name={} phase={}",
                    project.id,
                    project.declared_entity,
                    project.global_id.as_deref().unwrap_or("-"),
                    project.name.as_deref().unwrap_or("-"),
                    project.long_name.as_deref().unwrap_or("-"),
                    project.phase.as_deref().unwrap_or("-"),
                );
            }
        }
        "summary" => {
            let mut artifacts_root = default_ifc_artifacts_root();
            let mut model = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--artifacts-root" => {
                        artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    _ => return Err(format!("unknown option for summary: {arg}").into()),
                }
            }

            let model = model.ok_or("summary requires --model <slug>")?;
            let handle = VelrIfcModel::open(IfcArtifactLayout::new(&artifacts_root, model))?;
            let overview = handle.model_overview()?;
            println!("database: {}", overview.database.display());
            println!("node_count: {}", overview.node_count);
            println!("edge_count: {}", overview.edge_count);
            println!("project_count: {}", overview.projects.len());
            for project in overview.projects {
                println!(
                    "- {} [{}] {}",
                    project.id,
                    project.declared_entity,
                    project.name.as_deref().unwrap_or("-"),
                );
            }
        }
        "body-summary" => {
            let mut artifacts_root = default_ifc_artifacts_root();
            let mut model = None;
            let mut diagnostic = false;
            let mut brep_limit_items = None;
            let mut write_cache = false;
            let mut cache_diagnostic = false;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--artifacts-root" => {
                        artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    "--diagnostic" => diagnostic = true,
                    "--limit-brep-items" => {
                        let value = require_value(&mut args, &arg)?;
                        brep_limit_items = Some(value.parse::<usize>().map_err(|_| {
                            format!("--limit-brep-items must be a non-negative integer: {value}")
                        })?)
                    }
                    "--write-cache" => write_cache = true,
                    "--cache-diagnostic" => cache_diagnostic = true,
                    _ => return Err(format!("unknown option for body-summary: {arg}").into()),
                }
            }

            let model = model.ok_or("body-summary requires --model <slug>")?;
            if cache_diagnostic {
                let diagnostic = VelrIfcModel::diagnose_body_package_cache_from_artifacts_root(
                    &artifacts_root,
                    &model,
                )?;
                println!(
                    "geometry_cache_status: {}",
                    diagnostic.cache_status.as_str()
                );
                if let Some(bytes) = diagnostic.cache_bytes {
                    println!("cache_bytes: {bytes}");
                }
                if let Some(summary) = diagnostic.geometry_summary() {
                    println!("definitions: {}", summary.definitions);
                    println!("elements: {}", summary.elements);
                    println!("instances: {}", summary.instances);
                    println!("triangles: {}", summary.triangles);
                }
                for timing in diagnostic.timings {
                    println!("phase.{}.ms: {}", timing.name, timing.elapsed_ms);
                    if let Some(rows) = timing.rows {
                        println!("phase.{}.rows: {}", timing.name, rows);
                    }
                }
                return Ok(());
            }
            if diagnostic || brep_limit_items.is_some() || write_cache {
                let handle = VelrIfcModel::open(IfcArtifactLayout::new(&artifacts_root, model))?;
                let diagnostic = handle.build_body_package_diagnostic_with_cache_write(
                    brep_limit_items,
                    write_cache,
                )?;
                let summary = diagnostic.geometry_summary();
                let colored_instances = diagnostic
                    .instance_summaries()
                    .into_iter()
                    .filter(|instance| instance.display_color.is_some())
                    .count();
                println!("geometry_cache_status: diagnostic_uncached");
                println!(
                    "brep_limit_items: {}",
                    format_optional_usize(brep_limit_items)
                );
                println!("definitions: {}", summary.definitions);
                println!("elements: {}", summary.elements);
                println!("instances: {}", summary.instances);
                println!("colored_instances: {colored_instances}");
                println!("triangles: {}", summary.triangles);
                println!("brep_geometry_items: {}", diagnostic.brep.geometry_items);
                println!("brep_geometry_faces: {}", diagnostic.brep.geometry_faces);
                println!(
                    "brep_geometry_point_rows: {}",
                    diagnostic.brep.geometry_point_rows
                );
                println!("brep_metadata_rows: {}", diagnostic.brep.metadata_rows);
                println!("cache_written: {}", diagnostic.cache_written);
                for timing in diagnostic.timings {
                    println!("phase.{}.ms: {}", timing.name, timing.elapsed_ms);
                    if let Some(rows) = timing.rows {
                        println!("phase.{}.rows: {}", timing.name, rows);
                    }
                }
                return Ok(());
            }

            let load = VelrIfcModel::load_body_package_with_cache_status_from_artifacts_root(
                &artifacts_root,
                &model,
            )?;
            let summary = load.geometry_summary();
            let colored_instances = load
                .instance_summaries()
                .into_iter()
                .filter(|instance| instance.display_color.is_some())
                .count();
            println!("geometry_cache_status: {}", load.cache_status.as_str());
            println!("definitions: {}", summary.definitions);
            println!("elements: {}", summary.elements);
            println!("instances: {}", summary.instances);
            println!("colored_instances: {colored_instances}");
            println!("triangles: {}", summary.triangles);
        }
        "body-instances" => {
            let mut artifacts_root = default_ifc_artifacts_root();
            let mut model = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--artifacts-root" => {
                        artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    _ => return Err(format!("unknown option for body-instances: {arg}").into()),
                }
            }

            let model = model.ok_or("body-instances requires --model <slug>")?;
            let load = VelrIfcModel::load_body_package_with_cache_status_from_artifacts_root(
                &artifacts_root,
                &model,
            )?;
            println!("geometry_cache_status: {}", load.cache_status.as_str());
            for instance in load.instance_summaries() {
                let color = instance.display_color.map_or_else(
                    || "-".to_string(),
                    |color| {
                        let [red, green, blue] = color.as_rgb();
                        format!("({red:.3}, {green:.3}, {blue:.3})")
                    },
                );
                println!(
                    "instance_id={} definition_id={} label={} external_id={} color={} face_visibility={} center=({:.3}, {:.3}, {:.3}) size=({:.3}, {:.3}, {:.3}) min=({:.3}, {:.3}, {:.3}) max=({:.3}, {:.3}, {:.3})",
                    instance.instance_id,
                    instance.definition_id,
                    instance.label,
                    instance.external_id,
                    color,
                    match instance.face_visibility {
                        cc_w_types::FaceVisibility::OneSided => "one-sided",
                        cc_w_types::FaceVisibility::DoubleSided => "double-sided",
                    },
                    instance.bounds_center.x,
                    instance.bounds_center.y,
                    instance.bounds_center.z,
                    instance.bounds_size.x,
                    instance.bounds_size.y,
                    instance.bounds_size.z,
                    instance.bounds_min.x,
                    instance.bounds_min.y,
                    instance.bounds_min.z,
                    instance.bounds_max.x,
                    instance.bounds_max.y,
                    instance.bounds_max.z,
                );
            }
        }
        "placement-summary" => {
            let mut artifacts_root = default_ifc_artifacts_root();
            let mut model = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--artifacts-root" => {
                        artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    _ => return Err(format!("unknown option for placement-summary: {arg}").into()),
                }
            }

            let model = model.ok_or("placement-summary requires --model <slug>")?;
            let handle = VelrIfcModel::open(IfcArtifactLayout::new(&artifacts_root, model))?;
            let summary = handle.placement_summary()?;
            println!("local_placements: {}", summary.local_placements);
            println!(
                "placements_with_relative_placement: {}",
                summary.placements_with_relative_placement
            );
            println!(
                "placements_missing_relative_placement: {}",
                summary.placements_missing_relative_placement
            );
            println!("placements_with_parent: {}", summary.placements_with_parent);
        }
        "cypher" => {
            let mut artifacts_root = default_ifc_artifacts_root();
            let mut model = None;
            let mut query = None;

            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--model" => model = Some(require_value(&mut args, &arg)?),
                    "--query" => query = Some(require_value(&mut args, &arg)?),
                    "--artifacts-root" => {
                        artifacts_root = PathBuf::from(require_value(&mut args, &arg)?)
                    }
                    _ => return Err(format!("unknown option for cypher: {arg}").into()),
                }
            }

            let model = model.ok_or("cypher requires --model <slug>")?;
            let query = query.ok_or("cypher requires --query <openCypher>")?;
            let handle = VelrIfcModel::open(IfcArtifactLayout::new(&artifacts_root, model))?;
            let result = handle.execute_cypher_rows(&query)?;
            println!("{}", result.columns.join("\t"));
            for row in result.rows {
                println!("{}", row.join("\t"));
            }
        }
        _ => return Err(format!("unknown command: {command}").into()),
    }

    Ok(())
}

fn require_value(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    args.next()
        .ok_or_else(|| format!("missing value for {flag}").into())
}

fn format_optional_usize(value: Option<usize>) -> String {
    value.map_or_else(|| "all".to_string(), |value| value.to_string())
}

fn print_usage() {
    println!("cc-w-velr-tool commands:");
    println!("  list-fixtures");
    println!("  sync-fixtures [--velr-ifc-root <path>] [--fixtures-root <path>]");
    println!(
        "  import (--fixture <slug> | --ifc <path>) [--model <slug>] [--artifacts-root <path>] [--velr-ifc-root <path>] [--debug] [--debug-artifacts|--no-debug-artifacts] [--replace-existing]"
    );
    println!(
        "  import-fixtures [--artifacts-root <path>] [--velr-ifc-root <path>] [--debug] [--debug-artifacts|--no-debug-artifacts] [--replace-existing]"
    );
    println!("  clear-artifacts (--model <slug> | --all) [--artifacts-root <path>]");
    println!("  clear-geometry-cache (--model <slug> | --all) [--artifacts-root <path>]");
    println!("  clear-legacy-runtime (--model <slug> | --all) [--artifacts-root <path>]");
    println!(
        "  refresh-runtime (--model <slug> | --schema <ifc-schema>) [--artifacts-root <path>] [--velr-ifc-root <path>]"
    );
    println!("  query-projects --model <slug> [--artifacts-root <path>]");
    println!("  summary --model <slug> [--artifacts-root <path>]");
    println!(
        "  body-summary --model <slug> [--artifacts-root <path>] [--diagnostic] [--limit-brep-items <n>] [--write-cache] [--cache-diagnostic]"
    );
    println!("  body-instances --model <slug> [--artifacts-root <path>]");
    println!("  placement-summary --model <slug> [--artifacts-root <path>]");
    println!("  cypher --model <slug> --query <openCypher> [--artifacts-root <path>]");
}
