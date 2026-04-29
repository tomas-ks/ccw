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
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/js
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/styles
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/vendor/sigma
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/vendor/sigma-edge-curve
    cargo build -p cc-w-platform-web --lib --target wasm32-unknown-unknown
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
    cp -R crates/cc-w-platform-web/web/js/. crates/cc-w-platform-web/artifacts/viewer/js/
    cp -R crates/cc-w-platform-web/web/styles/. crates/cc-w-platform-web/artifacts/viewer/styles/
    cp crates/cc-w-platform-web/web/index.html crates/cc-w-platform-web/artifacts/viewer/index.html
    echo "web viewer output: crates/cc-w-platform-web/artifacts/viewer/index.html"

web-viewer-build-release:
    if [[ ! -d crates/cc-w-platform-web/web/node_modules/@xterm/xterm || ! -d crates/cc-w-platform-web/web/node_modules/sigma || ! -d crates/cc-w-platform-web/web/node_modules/graphology || ! -d crates/cc-w-platform-web/web/node_modules/@sigma/edge-curve ]]; then npm ci --prefix crates/cc-w-platform-web/web; fi
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/pkg
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/vendor
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/js
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/styles
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/vendor/sigma
    mkdir -p crates/cc-w-platform-web/artifacts/viewer/vendor/sigma-edge-curve
    cargo build --release -p cc-w-platform-web --lib --target wasm32-unknown-unknown
    wasm-bindgen --target web --no-typescript --out-dir crates/cc-w-platform-web/artifacts/viewer/pkg target/wasm32-unknown-unknown/release/cc_w_platform_web.wasm
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
    cp -R crates/cc-w-platform-web/web/js/. crates/cc-w-platform-web/artifacts/viewer/js/
    cp -R crates/cc-w-platform-web/web/styles/. crates/cc-w-platform-web/artifacts/viewer/styles/
    cp crates/cc-w-platform-web/web/index.html crates/cc-w-platform-web/artifacts/viewer/index.html
    echo "release web viewer output: crates/cc-w-platform-web/artifacts/viewer/index.html"

web-viewer:
    just web-viewer-build
    cargo build -p cc-w-platform-web --features native-server --bin cc-w-platform-web-cypher-worker
    cargo run -p cc-w-platform-web --features native-server --bin cc-w-platform-web-server -- --root crates/cc-w-platform-web/artifacts/viewer --port 8001

web-viewer-electron-build:
    just web-viewer-build
    if [[ ! -d crates/cc-w-platform-web/web/node_modules/electron ]]; then npm ci --prefix crates/cc-w-platform-web/web; fi
    cargo build -p cc-w-platform-web --features native-server --bin cc-w-platform-web-server --bin cc-w-platform-web-cypher-worker

web-viewer-electron-build-release:
    just web-viewer-build-release
    if [[ ! -d crates/cc-w-platform-web/web/node_modules/electron ]]; then npm ci --prefix crates/cc-w-platform-web/web; fi
    cargo build --release -p cc-w-platform-web --features native-server --bin cc-w-platform-web-server --bin cc-w-platform-web-cypher-worker

web-viewer-electron:
    just web-viewer-electron-build
    npm run electron --prefix crates/cc-w-platform-web/web

web-viewer-electron-release:
    just web-viewer-electron-build-release
    CC_W_WEB_SERVER_BINARY="$PWD/target/release/cc-w-platform-web-server" npm run electron --prefix crates/cc-w-platform-web/web

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

opencode-install:
    curl -fsSL https://opencode.ai/install | bash -s -- --no-modify-path
    mkdir -p .tools/opencode/bin .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    ln -sf "$HOME/.opencode/bin/opencode" .tools/opencode/bin/opencode
    XDG_CACHE_HOME="$PWD/.tools/opencode/cache" XDG_DATA_HOME="$PWD/.tools/opencode/data" XDG_CONFIG_HOME="$PWD/.tools/opencode/config" XDG_STATE_HOME="$PWD/.tools/opencode/state" .tools/opencode/bin/opencode --version
    echo "repo-local opencode launcher: .tools/opencode/bin/opencode"

opencode-check:
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    XDG_CACHE_HOME="$PWD/.tools/opencode/cache" XDG_DATA_HOME="$PWD/.tools/opencode/data" XDG_CONFIG_HOME="$PWD/.tools/opencode/config" XDG_STATE_HOME="$PWD/.tools/opencode/state" .tools/opencode/bin/opencode --version

opencode-login:
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/home .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    HOME="$PWD/.tools/opencode/home" XDG_CACHE_HOME="$PWD/.tools/opencode/cache" XDG_DATA_HOME="$PWD/.tools/opencode/data" XDG_CONFIG_HOME="$PWD/.tools/opencode/config" XDG_STATE_HOME="$PWD/.tools/opencode/state" OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" .tools/opencode/bin/opencode auth login

opencode-sync-agents:
    node tools/opencode/sync-agents.mjs

web-viewer-opencode: opencode-sync-agents
    just web-viewer-build
    cargo build -p cc-w-platform-web --features native-server --bin cc-w-platform-web-cypher-worker
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/home .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    # Model discovery stays narrowed by tools/opencode/provider-whitelist.json.
    # The repo-local agent defaults to ifc-explorer for OpenAI-like models and to
    # ifc-playbook-cypher-only for Gemma-like models unless CC_W_OPENCODE_AGENT overrides it.
    real_home="$HOME"; \
    model_default="${CC_W_OPENCODE_MODEL:-openai/gpt-5.4}"; \
    case "$model_default" in \
        ollama/gemma4:*) agent_default="${CC_W_OPENCODE_AGENT:-ifc-playbook-cypher-only}" ;; \
        *) agent_default="${CC_W_OPENCODE_AGENT:-ifc-explorer}" ;; \
    esac; \
    env HOME="$PWD/.tools/opencode/home" \
        CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
        RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
        XDG_CACHE_HOME="$PWD/.tools/opencode/cache" \
        XDG_DATA_HOME="$PWD/.tools/opencode/data" \
        XDG_CONFIG_HOME="$PWD/.tools/opencode/config" \
        XDG_STATE_HOME="$PWD/.tools/opencode/state" \
        OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_CYPHER_WORKER_BINARY="$PWD/target/debug/cc-w-platform-web-cypher-worker" \
        CC_W_AGENT_BACKEND=opencode \
        CC_W_OPENCODE_EXECUTABLE="$PWD/.tools/opencode/bin/opencode" \
        CC_W_OPENCODE_WORKDIR="$PWD" \
        CC_W_OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_OPENCODE_AGENT="$agent_default" \
        CC_W_OPENCODE_MODEL="$model_default" \
        CC_W_OPENCODE_DISCOVER_MODELS="${CC_W_OPENCODE_DISCOVER_MODELS:-1}" \
        CC_W_OPENCODE_TIMEOUT_MS="${CC_W_OPENCODE_TIMEOUT_MS:-180000}" \
        cargo run -p cc-w-platform-web --features native-server --bin cc-w-platform-web-server -- --root crates/cc-w-platform-web/artifacts/viewer --port 8001

web-viewer-opencode-electron: opencode-sync-agents
    just web-viewer-electron-build
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/home .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    # Electron shell around the same web viewer and OpenCode-backed server.
    real_home="$HOME"; \
    model_default="${CC_W_OPENCODE_MODEL:-openai/gpt-5.4}"; \
    case "$model_default" in \
        ollama/gemma4:*) agent_default="${CC_W_OPENCODE_AGENT:-ifc-playbook-cypher-only}" ;; \
        *) agent_default="${CC_W_OPENCODE_AGENT:-ifc-explorer}" ;; \
    esac; \
    env HOME="$PWD/.tools/opencode/home" \
        CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
        RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
        XDG_CACHE_HOME="$PWD/.tools/opencode/cache" \
        XDG_DATA_HOME="$PWD/.tools/opencode/data" \
        XDG_CONFIG_HOME="$PWD/.tools/opencode/config" \
        XDG_STATE_HOME="$PWD/.tools/opencode/state" \
        OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_CYPHER_WORKER_BINARY="$PWD/target/debug/cc-w-platform-web-cypher-worker" \
        CC_W_AGENT_BACKEND=opencode \
        CC_W_OPENCODE_EXECUTABLE="$PWD/.tools/opencode/bin/opencode" \
        CC_W_OPENCODE_WORKDIR="$PWD" \
        CC_W_OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_OPENCODE_AGENT="$agent_default" \
        CC_W_OPENCODE_MODEL="$model_default" \
        CC_W_OPENCODE_DISCOVER_MODELS="${CC_W_OPENCODE_DISCOVER_MODELS:-1}" \
        CC_W_OPENCODE_TIMEOUT_MS="${CC_W_OPENCODE_TIMEOUT_MS:-180000}" \
        npm run electron --prefix crates/cc-w-platform-web/web

web-viewer-opencode-electron-release: opencode-sync-agents
    just web-viewer-electron-build-release
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/home .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    # Electron shell around the release-built web viewer and OpenCode-backed server.
    real_home="$HOME"; \
    model_default="${CC_W_OPENCODE_MODEL:-openai/gpt-5.4}"; \
    case "$model_default" in \
        ollama/gemma4:*) agent_default="${CC_W_OPENCODE_AGENT:-ifc-playbook-cypher-only}" ;; \
        *) agent_default="${CC_W_OPENCODE_AGENT:-ifc-explorer}" ;; \
    esac; \
    env HOME="$PWD/.tools/opencode/home" \
        CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
        RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
        XDG_CACHE_HOME="$PWD/.tools/opencode/cache" \
        XDG_DATA_HOME="$PWD/.tools/opencode/data" \
        XDG_CONFIG_HOME="$PWD/.tools/opencode/config" \
        XDG_STATE_HOME="$PWD/.tools/opencode/state" \
        OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_WEB_SERVER_BINARY="$PWD/target/release/cc-w-platform-web-server" \
        CC_W_CYPHER_WORKER_BINARY="$PWD/target/release/cc-w-platform-web-cypher-worker" \
        CC_W_AGENT_BACKEND=opencode \
        CC_W_OPENCODE_EXECUTABLE="$PWD/.tools/opencode/bin/opencode" \
        CC_W_OPENCODE_WORKDIR="$PWD" \
        CC_W_OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_OPENCODE_AGENT="$agent_default" \
        CC_W_OPENCODE_MODEL="$model_default" \
        CC_W_OPENCODE_DISCOVER_MODELS="${CC_W_OPENCODE_DISCOVER_MODELS:-1}" \
        CC_W_OPENCODE_TIMEOUT_MS="${CC_W_OPENCODE_TIMEOUT_MS:-180000}" \
        npm run electron --prefix crates/cc-w-platform-web/web

web-viewer-opencode-strict: opencode-sync-agents
    just web-viewer-build
    cargo build -p cc-w-platform-web --features native-server --bin cc-w-platform-web-cypher-worker
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/home .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    # Strict IFC agent profile for Gemma-like models: canonical `ifc_*` tools only.
    real_home="$HOME"; \
    env HOME="$PWD/.tools/opencode/home" \
        CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
        RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
        XDG_CACHE_HOME="$PWD/.tools/opencode/cache" \
        XDG_DATA_HOME="$PWD/.tools/opencode/data" \
        XDG_CONFIG_HOME="$PWD/.tools/opencode/config" \
        XDG_STATE_HOME="$PWD/.tools/opencode/state" \
        OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_AGENT_BACKEND=opencode \
        CC_W_OPENCODE_EXECUTABLE="$PWD/.tools/opencode/bin/opencode" \
        CC_W_OPENCODE_WORKDIR="$PWD" \
        CC_W_OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_OPENCODE_AGENT="${CC_W_OPENCODE_AGENT:-ifc-explorer-strict}" \
        CC_W_OPENCODE_MODEL="${CC_W_OPENCODE_MODEL:-ollama/gemma4:e4b}" \
        CC_W_OPENCODE_DISCOVER_MODELS="${CC_W_OPENCODE_DISCOVER_MODELS:-1}" \
        CC_W_OPENCODE_TIMEOUT_MS="${CC_W_OPENCODE_TIMEOUT_MS:-180000}" \
        cargo run -p cc-w-platform-web --features native-server --bin cc-w-platform-web-server -- --root crates/cc-w-platform-web/artifacts/viewer --port 8001

web-viewer-opencode-42: opencode-sync-agents
    just web-viewer-build
    cargo build -p cc-w-platform-web --features native-server --bin cc-w-platform-web-cypher-worker
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/home .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    # Debug agent: literal `42` smoke test with no tools.
    real_home="$HOME"; \
    env HOME="$PWD/.tools/opencode/home" \
        CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
        RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
        XDG_CACHE_HOME="$PWD/.tools/opencode/cache" \
        XDG_DATA_HOME="$PWD/.tools/opencode/data" \
        XDG_CONFIG_HOME="$PWD/.tools/opencode/config" \
        XDG_STATE_HOME="$PWD/.tools/opencode/state" \
        OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_AGENT_BACKEND=opencode \
        CC_W_OPENCODE_EXECUTABLE="$PWD/.tools/opencode/bin/opencode" \
        CC_W_OPENCODE_WORKDIR="$PWD" \
        CC_W_OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_OPENCODE_AGENT="${CC_W_OPENCODE_AGENT:-ifc-answer-42}" \
        CC_W_OPENCODE_MODEL="${CC_W_OPENCODE_MODEL:-ollama/gemma4:e4b}" \
        CC_W_OPENCODE_DISCOVER_MODELS="${CC_W_OPENCODE_DISCOVER_MODELS:-1}" \
        CC_W_OPENCODE_TIMEOUT_MS="${CC_W_OPENCODE_TIMEOUT_MS:-180000}" \
        cargo run -p cc-w-platform-web --features native-server --bin cc-w-platform-web-server -- --root crates/cc-w-platform-web/artifacts/viewer --port 8001

web-viewer-opencode-cypher-only: opencode-sync-agents
    just web-viewer-build
    cargo build -p cc-w-platform-web --features native-server --bin cc-w-platform-web-cypher-worker
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/home .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    # Debug agent: only `ifc_readonly_cypher` is allowed.
    real_home="$HOME"; \
    env HOME="$PWD/.tools/opencode/home" \
        CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
        RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
        XDG_CACHE_HOME="$PWD/.tools/opencode/cache" \
        XDG_DATA_HOME="$PWD/.tools/opencode/data" \
        XDG_CONFIG_HOME="$PWD/.tools/opencode/config" \
        XDG_STATE_HOME="$PWD/.tools/opencode/state" \
        OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_AGENT_BACKEND=opencode \
        CC_W_OPENCODE_EXECUTABLE="$PWD/.tools/opencode/bin/opencode" \
        CC_W_OPENCODE_WORKDIR="$PWD" \
        CC_W_OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_OPENCODE_AGENT="${CC_W_OPENCODE_AGENT:-ifc-readonly-cypher-only}" \
        CC_W_OPENCODE_MODEL="${CC_W_OPENCODE_MODEL:-ollama/gemma4:e4b}" \
        CC_W_OPENCODE_DISCOVER_MODELS="${CC_W_OPENCODE_DISCOVER_MODELS:-1}" \
        CC_W_OPENCODE_TIMEOUT_MS="${CC_W_OPENCODE_TIMEOUT_MS:-180000}" \
        cargo run -p cc-w-platform-web --features native-server --bin cc-w-platform-web-server -- --root crates/cc-w-platform-web/artifacts/viewer --port 8001

web-viewer-opencode-playbook-cypher: opencode-sync-agents
    just web-viewer-build
    cargo build -p cc-w-platform-web --features native-server --bin cc-w-platform-web-cypher-worker
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/home .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    # Debug agent: only query playbooks and read-only Cypher are allowed.
    real_home="$HOME"; \
    env HOME="$PWD/.tools/opencode/home" \
        CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
        RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
        XDG_CACHE_HOME="$PWD/.tools/opencode/cache" \
        XDG_DATA_HOME="$PWD/.tools/opencode/data" \
        XDG_CONFIG_HOME="$PWD/.tools/opencode/config" \
        XDG_STATE_HOME="$PWD/.tools/opencode/state" \
        OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_AGENT_BACKEND=opencode \
        CC_W_OPENCODE_EXECUTABLE="$PWD/.tools/opencode/bin/opencode" \
        CC_W_OPENCODE_WORKDIR="$PWD" \
        CC_W_OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" \
        CC_W_OPENCODE_AGENT="${CC_W_OPENCODE_AGENT:-ifc-playbook-cypher-only}" \
        CC_W_OPENCODE_MODEL="${CC_W_OPENCODE_MODEL:-ollama/gemma4:e4b}" \
        CC_W_OPENCODE_DISCOVER_MODELS="${CC_W_OPENCODE_DISCOVER_MODELS:-1}" \
        CC_W_OPENCODE_TIMEOUT_MS="${CC_W_OPENCODE_TIMEOUT_MS:-180000}" \
        cargo run -p cc-w-platform-web --features native-server --bin cc-w-platform-web-server -- --root crates/cc-w-platform-web/artifacts/viewer --port 8001

opencode-smoke: opencode-sync-agents
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/home .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    log_file="$PWD/.tools/opencode/state/opencode-smoke.log"; \
    rm -f "$log_file"; \
    HOME="$PWD/.tools/opencode/home" XDG_CACHE_HOME="$PWD/.tools/opencode/cache" XDG_DATA_HOME="$PWD/.tools/opencode/data" XDG_CONFIG_HOME="$PWD/.tools/opencode/config" XDG_STATE_HOME="$PWD/.tools/opencode/state" OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" .tools/opencode/bin/opencode serve --pure --hostname 127.0.0.1 --port 0 >"$log_file" 2>&1 & \
    pid=$!; \
    for _ in $(seq 1 100); do \
        if grep -q "opencode server listening" "$log_file"; then \
            break; \
        fi; \
        sleep 0.1; \
    done; \
    if ! grep -q "opencode server listening" "$log_file"; then \
        cat "$log_file"; \
        kill "$pid" || true; \
        wait "$pid" || true; \
        exit 1; \
    fi; \
    kill "$pid" || true; \
    wait "$pid" || true; \
    cat "$log_file"

opencode-acp: opencode-sync-agents
    test -x .tools/opencode/bin/opencode
    mkdir -p .tools/opencode/home .tools/opencode/cache .tools/opencode/data .tools/opencode/config .tools/opencode/state
    HOME="$PWD/.tools/opencode/home" XDG_CACHE_HOME="$PWD/.tools/opencode/cache" XDG_DATA_HOME="$PWD/.tools/opencode/data" XDG_CONFIG_HOME="$PWD/.tools/opencode/config" XDG_STATE_HOME="$PWD/.tools/opencode/state" OPENCODE_CONFIG="$PWD/tools/opencode/opencode.json" CC_W_OPENCODE_ACP_HOSTNAME="${CC_W_OPENCODE_ACP_HOSTNAME:-127.0.0.1}" CC_W_OPENCODE_ACP_PORT="${CC_W_OPENCODE_ACP_PORT:-0}" .tools/opencode/bin/opencode acp --pure --hostname "${CC_W_OPENCODE_ACP_HOSTNAME:-127.0.0.1}" --port "${CC_W_OPENCODE_ACP_PORT:-0}"
