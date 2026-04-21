set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

visual-e2e:
    cargo test -p cc-w-platform-headless visual_suite_writes_artifacts_for_manual_review -- --ignored --nocapture
    echo "visual e2e output: crates/cc-w-platform-headless/artifacts/visual-e2e/index.html"

headless-snapshots:
    cargo run -p cc-w-platform-headless -- --snapshot-suite

headless-accept-snapshot case:
    cargo run -p cc-w-platform-headless -- --snapshot-suite --accept-snapshot "{{case}}"

headless-invalidate-snapshot case:
    cargo run -p cc-w-platform-headless -- --invalidate-snapshot "{{case}}"

ifc-list-fixtures:
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- list-fixtures

ifc-sync-fixtures:
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- sync-fixtures

ifc-import fixture="building-architecture":
    fixture_value="{{fixture}}"; \
    fixture_value="${fixture_value#fixture=}"; \
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- import --fixture "$fixture_value"

ifc-import-all:
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- import-fixtures

ifc-clear-artifacts model="building-architecture":
    model_value="{{model}}"; \
    model_value="${model_value#model=}"; \
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- clear-artifacts --model "$model_value"

ifc-clear-artifacts-all:
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- clear-artifacts --all

ifc-clear-geometry-cache model="building-architecture":
    model_value="{{model}}"; \
    model_value="${model_value#model=}"; \
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- clear-geometry-cache --model "$model_value"

ifc-clear-geometry-cache-all:
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- clear-geometry-cache --all

ifc-clear-legacy-runtime model="building-architecture":
    model_value="{{model}}"; \
    model_value="${model_value#model=}"; \
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- clear-legacy-runtime --model "$model_value"

ifc-clear-legacy-runtime-all:
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- clear-legacy-runtime --all

ifc-refresh-runtime model="building-architecture":
    model_value="{{model}}"; \
    model_value="${model_value#model=}"; \
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- refresh-runtime --model "$model_value"

ifc-refresh-runtime-schema schema="IFC4X3_ADD2":
    schema_value="{{schema}}"; \
    schema_value="${schema_value#schema=}"; \
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- refresh-runtime --schema "$schema_value"

ifc-body-summary model="building-architecture":
    model_value="{{model}}"; \
    model_value="${model_value#model=}"; \
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- body-summary --model "$model_value"

ifc-rebuild-geometry model="building-architecture":
    just ifc-clear-geometry-cache model="{{model}}"
    just ifc-body-summary model="{{model}}"

ifc-summary model="building-architecture":
    model_value="{{model}}"; \
    model_value="${model_value#model=}"; \
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- summary --model "$model_value"

ifc-projects model="building-architecture":
    model_value="{{model}}"; \
    model_value="${model_value#model=}"; \
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- query-projects --model "$model_value"

ifc-cypher model="building-architecture" query="MATCH (n:IfcProject) RETURN id(n) AS id ORDER BY id":
    model_value="{{model}}"; \
    model_value="${model_value#model=}"; \
    query_value="{{query}}"; \
    query_value="${query_value#query=}"; \
    cargo run -p cc-w-velr --bin cc-w-velr-tool -- cypher --model "$model_value" --query "$query_value"

ifc-headless-render model="building-architecture" output="/tmp/cc-w-ifc.png":
    model_value="{{model}}"; \
    model_value="${model_value#model=}"; \
    output_value="{{output}}"; \
    output_value="${output_value#output=}"; \
    cargo run -p cc-w-platform-headless -- --resource "ifc/$model_value" --output "$output_value"

ifc-native-viewer model="building-architecture":
    model_value="{{model}}"; \
    model_value="${model_value#model=}"; \
    cargo run -p cc-w-platform-native -- --resource "ifc/$model_value"

web-viewer-build:
    if [[ ! -d crates/cc-w-platform-web/web/node_modules/@xterm/xterm || ! -d crates/cc-w-platform-web/web/node_modules/sigma || ! -d crates/cc-w-platform-web/web/node_modules/graphology || ! -d crates/cc-w-platform-web/web/node_modules/@sigma/edge-curve ]]; then npm ci --prefix crates/cc-w-platform-web/web; fi
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/pkg
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/vendor
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/vendor/sigma
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/vendor/sigma-edge-curve
    cargo build -p cc-w-platform-web --target wasm32-unknown-unknown
    wasm-bindgen --target web --no-typescript --out-dir crates/cc-w-platform-web/artifacts/viewer/pkg target/wasm32-unknown-unknown/debug/cc_w_platform_web.wasm
    cp crates/cc-w-platform-web/web/node_modules/@xterm/xterm/css/xterm.css crates/cc-w-platform-web/artifacts/viewer/vendor/xterm.css
    cp crates/cc-w-platform-web/web/node_modules/@xterm/xterm/lib/xterm.mjs crates/cc-w-platform-web/artifacts/viewer/vendor/xterm.mjs
    cp crates/cc-w-platform-web/web/node_modules/@xterm/addon-fit/lib/addon-fit.mjs crates/cc-w-platform-web/artifacts/viewer/vendor/addon-fit.mjs
    cp crates/cc-w-platform-web/web/node_modules/graphology/dist/graphology.umd.min.js crates/cc-w-platform-web/artifacts/viewer/vendor/graphology.js
    cp crates/cc-w-platform-web/web/node_modules/sigma/dist/sigma.min.js crates/cc-w-platform-web/artifacts/viewer/vendor/sigma.js
    cp -R crates/cc-w-platform-web/web/node_modules/sigma/dist crates/cc-w-platform-web/artifacts/viewer/vendor/sigma/
    cp -R crates/cc-w-platform-web/web/node_modules/sigma/rendering crates/cc-w-platform-web/artifacts/viewer/vendor/sigma/
    cp -R crates/cc-w-platform-web/web/node_modules/sigma/utils crates/cc-w-platform-web/artifacts/viewer/vendor/sigma/
    cp -R crates/cc-w-platform-web/web/node_modules/@sigma/edge-curve/dist crates/cc-w-platform-web/artifacts/viewer/vendor/sigma-edge-curve/
    cp crates/cc-w-platform-web/web/vendor/graphology-utils-is-graph.mjs crates/cc-w-platform-web/artifacts/viewer/vendor/graphology-utils-is-graph.mjs
    cp crates/cc-w-platform-web/web/index.html crates/cc-w-platform-web/artifacts/viewer/index.html
    echo "web viewer output: crates/cc-w-platform-web/artifacts/viewer/index.html"

web-viewer:
    just web-viewer-build
    cargo run -p cc-w-platform-web --features native-server --bin cc-w-platform-web-server -- --root crates/cc-w-platform-web/artifacts/viewer --port 8001

native-viewer resource="demo/pentagon":
    resource_value="{{resource}}"; \
    resource_value="${resource_value#resource=}"; \
    cargo run -p cc-w-platform-native -- --resource "$resource_value"

native-viewer-smoke resource="demo/pentagon" frames="5":
    resource_value="{{resource}}"; \
    resource_value="${resource_value#resource=}"; \
    env CC_W_AUTO_EXIT_FRAMES={{frames}} cargo run -p cc-w-platform-native -- --resource "$resource_value"

web-viewer-stop:
    pids=$(lsof -t -n -P -iTCP:8001-8032 -sTCP:LISTEN -c cc-w-plat 2>/dev/null | tr '\n' ' ' | xargs); \
    if [[ -z "$pids" ]]; then \
        echo "no running w web viewer servers"; \
    else \
        echo "stopping w web viewer servers: $pids"; \
        kill $pids; \
    fi
