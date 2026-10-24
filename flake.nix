{
  description = "Small Rust CLI toolbox";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];

      forAllSystems = nixpkgs.lib.genAttrs systems;

      pkgsFor = system: import nixpkgs {
        inherit system;
      };
    in {
      packages = forAllSystems (system:
        let
          pkgs = pkgsFor system;
        in {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "cli-toolbox";
            version = "dev";
            src = self;

            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [ pkgs.installShellFiles ];
            nativeCheckInputs = [ pkgs.git ];

            doCheck = true;

            postInstall = ''
              installShellCompletion --cmd cli-toolbox \
                --bash <($out/bin/cli-toolbox completion -c bash) \
                --zsh <($out/bin/cli-toolbox completion -c zsh) \
                --fish <($out/bin/cli-toolbox completion -c fish)
            '';
          };
        });

      devShells = forAllSystems (system:
        let
          pkgs = pkgsFor system;
        in {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default ];
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer
              lldb
              just
              git
              nil
              nixfmt
              statix
            ];
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          };
        });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/cli-toolbox";
        };
      });
    };
}
