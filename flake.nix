{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        openclaw = import ./openclaw.nix { inherit pkgs; };
        rustPackages = [
          pkgs.rustup
          pkgs.rust-analyzer
          pkgs.pkg-config
          pkgs.protobuf
          pkgs.zlib
          pkgs.openssl
          pkgs.llvmPackages.clang
          pkgs.llvmPackages.libclang
          pkgs.mold
          pkgs.gcc.cc.lib
        ];
        rustShellHook = ''
          if [ -f rust-toolchain.toml ]; then
            rust_version=$(grep 'channel' rust-toolchain.toml | cut -d '"' -f 2)
            rustup override set "$rust_version"
            rustup component add rust-src --toolchain "$rust_version" 2>/dev/null || true
            rustup component add rust-analyzer --toolchain "$rust_version" 2>/dev/null || true
          fi
        '';
        mkPowdShell = extraPackages: extraHook:
          pkgs.mkShell {
            packages = rustPackages ++ extraPackages;

            shellHook = rustShellHook + extraHook;

            RUSTFLAGS = "-C link-arg=-fuse-ld=mold";
            # Work around librocksdb-sys + GCC 15 header issue in trace_record.cc.
            CXXFLAGS = "-include cstdint";
            CARGO_INCREMENTAL = "1";
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            LD_LIBRARY_PATH = "${pkgs.zlib}/lib:${pkgs.gcc.cc.lib}/lib:${pkgs.openssl.out}/lib";
            OPENSSL_NO_VENDOR = "1";
          };
      in {
        packages = {
          openclaw = openclaw.wrapper;
          openclaw-bootstrap = openclaw.bootstrap;
        };

        devShells.default = mkPowdShell [] "";
        devShells.openclaw = mkPowdShell [
          pkgs.curl
          pkgs.git
          pkgs.jq
          openclaw.nodejs
          openclaw.pnpm
          openclaw.bootstrap
          openclaw.wrapper
        ] ''
          export POWD_REPO_ROOT="$PWD"
          export POWD_OPENCLAW_ROOT="$PWD/.tmp/openclaw"
          export OPENCLAW_HOME="$POWD_OPENCLAW_ROOT/home"
          export XDG_CONFIG_HOME="$POWD_OPENCLAW_ROOT/xdg-config"
          export XDG_STATE_HOME="$POWD_OPENCLAW_ROOT/xdg-state"
          mkdir -p "$POWD_OPENCLAW_ROOT" "$XDG_CONFIG_HOME" "$XDG_STATE_HOME"
        '';
      });
}
