{
  inputs = {
    # Pinned: needs Cargo 1.85+ (edition2024) and gcc < 15 (musl static libstdc++)
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
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

        cargoHash = "sha256-w362lKg46fJ//PzZsI4NvbqnIVTUhhMBUNt6NItxCcU=";

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
          # CMake 4.x removed compat with cmake_minimum_required < 3.5;
          # audiopus_sys bundles an old opus CMakeLists.txt that needs this.
          CMAKE_POLICY_VERSION_MINIMUM = "3.5";
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
          musl = let
            muslPkgs = pkgs.pkgsCross.musl64;
            gccLib = "${muslPkgs.stdenv.cc.cc}/x86_64-unknown-linux-musl/lib";
          in muslPkgs.rustPlatform.buildRustPackage (commonEnv // {
            pname = "${pname}-musl";
            inherit version;
            src = ./.;
            inherit cargoHash;

            nativeBuildInputs = commonNativeBuildInputs;
            buildInputs = [ muslPkgs.pkgsStatic.libopus ];

            CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
            PKG_CONFIG_ALL_STATIC = "1";

            # Force fully static binary: shadow dynamic libstdc++/libgcc_s with
            # linker scripts that redirect to static archives. This avoids the
            # gcc 15 PIE/libstdc++.a incompatibility when using -static.
            # See: https://github.com/NixOS/nixpkgs/issues/425367
            preBuild = ''
              OVERRIDE=$TMPDIR/force-static
              mkdir -p $OVERRIDE
              echo "INPUT(${gccLib}/libstdc++.a)" > $OVERRIDE/libstdc++.so
              echo "INPUT(${gccLib}/libstdc++.a)" > $OVERRIDE/libstdc++.so.6
              echo "INPUT(${gccLib}/libgcc_s.a)"  > $OVERRIDE/libgcc_s.so
              echo "INPUT(${gccLib}/libgcc_s.a)"  > $OVERRIDE/libgcc_s.so.1
              export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-feature=+crt-static -C relocation-model=static -C link-args=-L$OVERRIDE -C link-args=-lc"
            '';

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
          windows = let
            winPkgs = pkgs.pkgsCross.mingwW64;
            cc = winPkgs.stdenv.cc;
          in winPkgs.rustPlatform.buildRustPackage (commonEnv // {
            pname = "${pname}-windows";
            inherit version;
            src = ./.;
            inherit cargoHash;

            nativeBuildInputs = commonNativeBuildInputs;
            buildInputs = [
              winPkgs.windows.pthreads
            ];

            CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
            CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L ${winPkgs.windows.pthreads}/lib";

            # Provide mingw headers to bindgen so it can generate correct bindings
            BINDGEN_EXTRA_CLANG_ARGS = "--sysroot=${cc.cc}/x86_64-w64-mingw32 -idirafter ${cc.cc}/x86_64-w64-mingw32/include";

            env = {
              PKG_CONFIG_ALLOW_CROSS = "1";
            };

            meta = with pkgs.lib; {
              description = "Speech-to-text MCP server powered by whisper.cpp (Windows)";
              license = licenses.mit;
            };
          });
        };
      }
    );
}
