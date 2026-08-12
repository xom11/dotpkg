{
  description = "Declarative package management for Windows: winget and scoop from one dotfile";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages = rec {
          dotpkg = pkgs.callPackage ./nix/package.nix { };
          default = dotpkg;
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
          packages = with pkgs; [
            rustfmt
            clippy
            rust-analyzer
            # The two CI gates outside the suite, so a shell can run what CI
            # runs: scripts/check-citations.py and scripts/check-ps1-style.py.
            python3
          ];
        };

        # `nix run .#` runs `dotpkg` with whatever args follow. Useful on a
        # machine with winget or scoop; on one with neither it will find
        # nothing to manage, which is the honest answer rather than an error.
        apps.default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/dotpkg";
        };
      }
    )
    // {
      # Overlay other flakes / configs can add to nixpkgs.overlays.
      overlays.default = final: prev: {
        dotpkg = final.callPackage ./nix/package.nix { };
      };
    };
}
