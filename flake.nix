{
  description = "Doghouse: ScottyLabs project boards";
  nixConfig = {
    extra-substituters = [ "https://scottylabs.cachix.org" ];
    extra-trusted-public-keys = [
      "scottylabs.cachix.org-1:hajjEX5SLi/Y7yYloiXTt2IOr3towcTGRhMh1vu6Tjg="
    ];
  };
  inputs = {
    scottylabs.url = "git+https://git.cmu.dev/ScottyLabs/kennel";
    collar = {
      url = "git+https://git.cmu.dev/ScottyLabs/collar?ref=main&rev=cc937a82735ef8f2454aea8e0bd4e2a1ec72bebd";
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
            project.name = "doghouse";
            rust.enable = true;
            shell.packages = [
              collar
              pkgs.tmux
            ];
            shell.scripts.doghouse = ''
              export DOGHOUSE_COLLAR="${collar}/bin/collar"
              cd "$SCOTTYLABS_ROOT"
              exec cargo run -- "$@"
            '';
          };
          packages.default = helpers.buildRustService {
            src = ./.;
            nativeBuildInputs = [ pkgs.makeWrapper ];
            buildArgs.postInstall = ''
              wrapProgram $out/bin/doghouse --set-default DOGHOUSE_COLLAR ${collar}/bin/collar --prefix PATH : ${
                pkgs.lib.makeBinPath [ pkgs.tmux ]
              }
            '';
          };
        };
    };
}
