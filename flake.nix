{
  description = "Small Go CLI toolbox";

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
          default = pkgs.buildGoModule {
            pname = "cli-toolbox";
            version = "dev";
            src = self;

            vendorHash = "sha256-a4U0pFczR4Lg3ngTKlS1LXkPQclqfYX9UZ1/w9v1ZIE=";

            nativeBuildInputs = [ pkgs.installShellFiles ];

            doCheck = true;

            postInstall = ''
              installShellCompletion --cmd cli-toolbox \
                --bash <($out/bin/cli-toolbox completion -c bash) \
                --zsh <($out/bin/cli-toolbox completion -c zsh) \
                --fish <($out/bin/cli-toolbox completion -c fish)
            '';
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
