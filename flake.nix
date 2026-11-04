{
  description = "Mumble local-first voice dictation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self, nixpkgs, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      package = pkgs: self.packages.${pkgs.stdenv.hostPlatform.system}.mumble;
    in
    {
      packages = forSystems (
        pkgs:
        let
          mumble = pkgs.callPackage ./nix/package.nix { };
        in
        {
          inherit mumble;
          default = mumble;
        }
      );

      overlays.default = final: _prev: {
        mumble = final.callPackage ./nix/package.nix { };
      };

      nixosModules.mumble =
        { pkgs, ... }:
        {
          imports = [ ./nix/module.nix ];
          programs.mumble.package = nixpkgs.lib.mkDefault (package pkgs);
        };
      nixosModules.default = self.nixosModules.mumble;

      homeManagerModules.mumble =
        { pkgs, ... }:
        {
          imports = [ ./nix/hm-module.nix ];
          programs.mumble.package = nixpkgs.lib.mkDefault (package pkgs);
        };
      homeManagerModules.default = self.homeManagerModules.mumble;

      devShells = forSystems (pkgs: {
        default = (package pkgs).devShell;
      });
      checks = forSystems (pkgs: import ./nix/checks.nix { inherit self pkgs; });
      formatter = forSystems (pkgs: pkgs.nixfmt);
    };
}
