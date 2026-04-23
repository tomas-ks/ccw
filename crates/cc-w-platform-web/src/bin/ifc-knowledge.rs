#[path = "server/agent_executor.rs"]
mod agent_executor;
#[path = "server/schema_reference.rs"]
mod schema_reference;

use std::{
    env,
    path::PathBuf,
    process::ExitCode,
};

use cc_w_velr::{default_ifc_artifacts_root, IfcSchemaId};
use schema_reference::{
    load_entity_references, load_query_playbooks, load_relation_references, load_schema_context,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse(env::args().skip(1))?;
    let schema = IfcSchemaId::parse(&cli.schema);
    let artifacts_root = cli
        .artifacts_root
        .unwrap_or_else(default_ifc_artifacts_root);

    let output = match cli.command {
        Command::SchemaContext => serde_json::to_value(load_schema_context(&artifacts_root, &schema)?)
            .map_err(|error| format!("could not serialize schema context: {error}"))?,
        Command::EntityReference { entity_names } => serde_json::to_value(load_entity_references(
            &artifacts_root,
            &schema,
            &entity_names,
        )?)
        .map_err(|error| format!("could not serialize entity references: {error}"))?,
        Command::QueryPlaybook { goal, entity_names } => serde_json::to_value(load_query_playbooks(
            &artifacts_root,
            &schema,
            &goal,
            &entity_names,
        )?)
        .map_err(|error| format!("could not serialize query playbooks: {error}"))?,
        Command::RelationReference { relation_names } => serde_json::to_value(
            load_relation_references(&artifacts_root, &schema, &relation_names)?,
        )
        .map_err(|error| format!("could not serialize relation references: {error}"))?,
    };

    let json = serde_json::to_string_pretty(&output)
        .map_err(|error| format!("could not format JSON output: {error}"))?;
    println!("{json}");
    Ok(())
}

#[derive(Debug, Clone)]
struct Cli {
    artifacts_root: Option<PathBuf>,
    schema: String,
    command: Command,
}

#[derive(Debug, Clone)]
enum Command {
    SchemaContext,
    EntityReference { entity_names: Vec<String> },
    QueryPlaybook { goal: String, entity_names: Vec<String> },
    RelationReference { relation_names: Vec<String> },
}

impl Cli {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut artifacts_root = None;
        let mut schema = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--artifacts-root" => {
                    let value = args.next().ok_or_else(|| {
                        "--artifacts-root expects a path".to_owned()
                    })?;
                    artifacts_root = Some(PathBuf::from(value));
                }
                "--schema" => {
                    let value = args.next().ok_or_else(|| {
                        "--schema expects an IFC schema id".to_owned()
                    })?;
                    schema = Some(value);
                }
                "--help" | "-h" => return Err(usage()),
                "schema-context" => {
                    return Ok(Self {
                        artifacts_root,
                        schema: schema.unwrap_or_else(|| "IFC4X3_ADD2".to_owned()),
                        command: Command::SchemaContext,
                    });
                }
                "entity-reference" => {
                    let entity_names = collect_multi_value_args(&mut args, "--entity")?;
                    return Ok(Self {
                        artifacts_root,
                        schema: schema.unwrap_or_else(|| "IFC4X3_ADD2".to_owned()),
                        command: Command::EntityReference { entity_names },
                    });
                }
                "query-playbook" => {
                    let mut goal = None;
                    let mut entity_names = Vec::new();
                    while let Some(next) = args.next() {
                        match next.as_str() {
                            "--goal" => {
                                goal = Some(args.next().ok_or_else(|| {
                                    "--goal expects a short description".to_owned()
                                })?);
                            }
                            "--entity" => {
                                entity_names.push(args.next().ok_or_else(|| {
                                    "--entity expects an IFC entity name".to_owned()
                                })?);
                            }
                            "--help" | "-h" => return Err(usage()),
                            other => {
                                return Err(format!(
                                    "unexpected argument `{other}` for query-playbook\n{}",
                                    usage()
                                ))
                            }
                        }
                    }
                    return Ok(Self {
                        artifacts_root,
                        schema: schema.unwrap_or_else(|| "IFC4X3_ADD2".to_owned()),
                        command: Command::QueryPlaybook {
                            goal: goal.ok_or_else(|| "--goal is required".to_owned())?,
                            entity_names,
                        },
                    });
                }
                "relation-reference" => {
                    let relation_names = collect_multi_value_args(&mut args, "--relation")?;
                    return Ok(Self {
                        artifacts_root,
                        schema: schema.unwrap_or_else(|| "IFC4X3_ADD2".to_owned()),
                        command: Command::RelationReference { relation_names },
                    });
                }
                other => {
                    return Err(format!(
                        "unexpected argument `{other}`\n{}",
                        usage()
                    ));
                }
            }
        }

        Err(usage())
    }
}

fn collect_multi_value_args(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    while let Some(next) = args.next() {
        match next.as_str() {
            "--entity" | "--relation" => {
                values.push(
                    args.next()
                        .ok_or_else(|| format!("{flag} expects a value"))?,
                );
            }
            "--help" | "-h" => return Err(usage()),
            other => {
                return Err(format!("unexpected argument `{other}`\n{}", usage()));
            }
        }
    }
    Ok(values)
}

fn usage() -> String {
    [
        "usage:",
        "  ifc-knowledge [--artifacts-root PATH] [--schema IFC4X3_ADD2] schema-context",
        "  ifc-knowledge [--artifacts-root PATH] [--schema IFC4X3_ADD2] entity-reference --entity IfcSlab [--entity IfcRoof ...]",
        "  ifc-knowledge [--artifacts-root PATH] [--schema IFC4X3_ADD2] query-playbook --goal \"hide the roof\" [--entity IfcRoof ...]",
        "  ifc-knowledge [--artifacts-root PATH] [--schema IFC4X3_ADD2] relation-reference --relation IfcRelAggregates [--relation RELATED_OBJECTS ...]",
    ]
    .join("\n")
}
