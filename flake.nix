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

            vendorHash = "sha256-7t9ZaHHX2ECoC+qJvOuMV9b4IiBy+iS6GcyOZO7ptNQ=";

            doCheck = true;
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
