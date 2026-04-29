use std::{
    error::Error,
    io::{self, Read},
    path::PathBuf,
    process,
    time::{Duration, Instant},
};

use cc_w_velr::{IfcArtifactLayout, VelrIfcModel};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CypherWorkerRequest {
    artifacts_root: PathBuf,
    model_slug: String,
    cypher: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CypherWorkerResponse {
    Ok {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        open_ms: u64,
        query_ms: u64,
    },
    Error {
        error: String,
    },
}

fn main() {
    let response = match run() {
        Ok(response) => response,
        Err(error) => CypherWorkerResponse::Error {
            error: error.to_string(),
        },
    };
    let ok = matches!(response, CypherWorkerResponse::Ok { .. });
    match serde_json::to_string(&response) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("could not serialize Cypher worker response: {error}");
            process::exit(2);
        }
    }
    if !ok {
        process::exit(1);
    }
}

fn run() -> Result<CypherWorkerResponse, Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let request: CypherWorkerRequest = serde_json::from_str(&input)?;

    let open_started = Instant::now();
    let model = VelrIfcModel::open(IfcArtifactLayout::new(
        &request.artifacts_root,
        &request.model_slug,
    ))?;
    let open_ms = elapsed_ms(open_started.elapsed());

    let query_started = Instant::now();
    let result = model.execute_cypher_rows(&request.cypher)?;
    let query_ms = elapsed_ms(query_started.elapsed());

    Ok(CypherWorkerResponse::Ok {
        columns: result.columns,
        rows: result.rows,
        open_ms,
        query_ms,
    })
}

fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
