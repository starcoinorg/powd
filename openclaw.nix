{ pkgs }:
let
  version = "2026.4.8";
  rev = "v${version}";
  nodejs = pkgs.nodejs_24;
  pnpm = pkgs.pnpm;
  source = pkgs.fetchFromGitHub {
    owner = "openclaw";
    repo = "openclaw";
    inherit rev;
    sha256 = "0yvryb2yv58p5bsc0q322mccvgngs2zs6qmm0jj2zjk1llinzlb3";
  };
  bootstrap = pkgs.writeShellScriptBin "openclaw-bootstrap" ''
    set -euo pipefail

    repo_root="''${POWD_REPO_ROOT:-$PWD}"
    if [ ! -f "$repo_root/flake.nix" ] || [ ! -e "$repo_root/.git" ]; then
      echo "set POWD_REPO_ROOT to the powd repository root before running openclaw-bootstrap" >&2
      exit 1
    fi

    root="''${POWD_OPENCLAW_ROOT:-$repo_root/.tmp/openclaw}"
    workspace="$root/workspace-${version}"
    marker="$workspace/.powd-openclaw-built"

    mkdir -p "$root"
    if [ ! -d "$workspace" ]; then
      rm -rf "$workspace"
      cp -R ${source} "$workspace"
      chmod -R u+w "$workspace"
    fi

    if [ ! -f "$marker" ]; then
      mkdir -p \
        "$root/home" \
        "$root/npm-cache" \
        "$root/pnpm-store" \
        "$root/xdg-cache"
      (
        cd "$workspace"
        export HOME="$root/home"
        export XDG_CACHE_HOME="$root/xdg-cache"
        export npm_config_cache="$root/npm-cache"
        ${pnpm}/bin/pnpm install --frozen-lockfile --store-dir "$root/pnpm-store" 1>&2
        ${pnpm}/bin/pnpm build 1>&2
        touch "$marker"
      )
    fi

    printf '%s\n' "$workspace"
  '';
  wrapper = pkgs.writeShellScriptBin "openclaw" ''
    set -euo pipefail
    workspace="$(${bootstrap}/bin/openclaw-bootstrap)"
    exec ${nodejs}/bin/node "$workspace/openclaw.mjs" "$@"
  '';
in {
  inherit bootstrap nodejs pnpm rev source version wrapper;
}
