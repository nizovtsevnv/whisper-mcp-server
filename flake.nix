{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        cudaPkgs = import nixpkgs {
          inherit system overlays;
          config.allowUnfree = true;
        };

        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        pname = cargoToml.package.name;
        version = cargoToml.package.version;

        cargoHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [
            "x86_64-unknown-linux-musl"
            "x86_64-pc-windows-gnu"
          ];
        };

        # Common native build inputs for whisper.cpp (cmake) and bindgen (libclang)
        commonNativeBuildInputs = with pkgs; [
          cmake
          pkg-config
          llvmPackages.libclang
        ];

        commonEnv = {
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
        };
      in {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            rustToolchain
            pkgs.cmake
            pkgs.pkg-config
            pkgs.libclang
            pkgs.llvmPackages.libclang
            pkgs.libopus
          ];

          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          shellHook = ''
            # Set up git hooks if .git exists
            if [ -d .git ]; then
              mkdir -p .git/hooks
              cat > .git/hooks/pre-commit << 'HOOK'
#!/usr/bin/env bash
set -e
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test
HOOK
              chmod +x .git/hooks/pre-commit
            fi
          '';
        };

        packages = {
          # Native linux build (glibc)
          default = pkgs.rustPlatform.buildRustPackage (commonEnv // {
            inherit pname version;
            src = ./.;
            inherit cargoHash;

            nativeBuildInputs = commonNativeBuildInputs;
            buildInputs = [ pkgs.libopus ];

            meta = with pkgs.lib; {
              description = "Speech-to-text MCP server powered by whisper.cpp";
              license = licenses.mit;
            };
          });

          # Static linux build (musl)
          musl = pkgs.pkgsStatic.rustPlatform.buildRustPackage (commonEnv // {
            pname = "${pname}-musl";
            inherit version;
            src = ./.;
            inherit cargoHash;

            nativeBuildInputs = commonNativeBuildInputs;
            buildInputs = [ pkgs.pkgsStatic.libopus ];

            CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
            PKG_CONFIG_ALL_STATIC = "1";

            meta = with pkgs.lib; {
              description = "Speech-to-text MCP server powered by whisper.cpp (musl static)";
              license = licenses.mit;
            };
          });

          # CUDA build (requires unfree NVIDIA packages)
          cuda = cudaPkgs.rustPlatform.buildRustPackage (commonEnv // {
            pname = "${pname}-cuda";
            inherit version;
            src = ./.;
            inherit cargoHash;

            buildFeatures = [ "cuda" ];

            nativeBuildInputs = commonNativeBuildInputs ++ [
              cudaPkgs.cudaPackages.cuda_nvcc
            ];
            buildInputs = [
              cudaPkgs.libopus
              cudaPkgs.cudaPackages.cuda_cudart
              cudaPkgs.cudaPackages.libcublas
            ];

            meta = with cudaPkgs.lib; {
              description = "Speech-to-text MCP server powered by whisper.cpp (CUDA)";
              license = licenses.mit;
            };
          });

          # Windows cross-compilation
          windows = pkgs.pkgsCross.mingwW64.rustPlatform.buildRustPackage (commonEnv // {
            pname = "${pname}-windows";
            inherit version;
            src = ./.;
            inherit cargoHash;

            nativeBuildInputs = commonNativeBuildInputs;
            buildInputs = [
              pkgs.pkgsCross.mingwW64.windows.pthreads
            ];

            CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
            PKG_CONFIG_ALLOW_CROSS = "1";
            CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L ${pkgs.pkgsCross.mingwW64.windows.pthreads}/lib";

            meta = with pkgs.lib; {
              description = "Speech-to-text MCP server powered by whisper.cpp (Windows)";
              license = licenses.mit;
            };
          });
        };
      }
    );
}
