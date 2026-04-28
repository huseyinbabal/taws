{
  description = "taws - Terminal UI for AWS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f {
        inherit system;
        pkgs = nixpkgs.legacyPackages.${system};
      });
    in
    {
      packages = forAllSystems ({ pkgs, ... }: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "taws";
          version = "1.3.0-rc.7";
          src = ./.;
          cargoHash = "sha256-7zZ2JJVQem2R072sefv2oB9mmQcRuUHVKKcb+HEnm6Y=";
          meta = {
            description = "Terminal UI for AWS - navigate, observe, and manage AWS resources";
            homepage = "https://github.com/huseyinbabal/taws";
            license = pkgs.lib.licenses.mit;
            mainProgram = "taws";
          };
        };
      });

      apps = forAllSystems ({ system, ... }: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/taws";
          meta.description = "Terminal UI for AWS";
        };
      });

      devShells = forAllSystems ({ pkgs, ... }: {
        default = pkgs.mkShell {
          buildInputs = with pkgs; [
            cargo
            rustc
            rust-analyzer
            rustfmt
            clippy
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
    };
}
