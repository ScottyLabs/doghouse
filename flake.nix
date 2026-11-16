{
  description = "Shepherd: ScottyLabs project boards";
  nixConfig = {
    extra-substituters = [ "https://scottylabs.cachix.org" ];
    extra-trusted-public-keys = [
      "scottylabs.cachix.org-1:hajjEX5SLi/Y7yYloiXTt2IOr3towcTGRhMh1vu6Tjg="
    ];
  };
  inputs = {
    scottylabs.url = "git+https://git.cmu.dev/ScottyLabs/kennel";
    collar = {
      url = "git+https://git.cmu.dev/ScottyLabs/collar?ref=fix/governance-schema&rev=4e46cbee16d048af1c2179740893704be0b7b5c3";
      inputs = {
        scottylabs.follows = "scottylabs";
        nixpkgs.follows = "nixpkgs";
        flake-parts.follows = "flake-parts";
      };
    };
    nixpkgs.follows = "scottylabs/nixpkgs";
    flake-parts.follows = "scottylabs/flake-parts";
  };
  outputs =
    inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [ inputs.scottylabs.flakeModules.default ];
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      perSystem =
        {
          helpers,
          pkgs,
          system,
          ...
        }:
        let
          collar = inputs.collar.packages.${system}.default;
        in
        {
          scottylabs = {
            project.name = "shepherd";
            rust.enable = true;
            shell.packages = [ collar ];
            shell.scripts.shepherd = ''
              export SHEPHERD_COLLAR="${collar}/bin/collar"
              cd "$SCOTTYLABS_ROOT"
              exec cargo run -- "$@"
            '';
          };
          packages.default = helpers.buildRustService {
            src = ./.;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            buildArgs.postInstall = ''
              wrapProgram $out/bin/shepherd --set-default SHEPHERD_COLLAR ${collar}/bin/collar
            '';
          };
        };
    };
}
