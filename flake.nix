{
  description = "A utility to shrink your media files";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      crane,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        craneLib = crane.mkLib pkgs;

        shrink-ray = craneLib.buildPackage {
          src = ./.;
          nativeBuildInputs = with pkgs; [ file ];
        };
      in
      {
        packages = {
          inherit shrink-ray;
          default = shrink-ray;
        };

        apps = {
          shrink-ray = flake-utils.lib.mkApp { drv = shrink-ray; };
        };

        devShells.default = craneLib.devShell {
          inputsFrom = [ shrink-ray ];
          packages = with pkgs; [ rust-analyzer ];
        };
        formatter = pkgs.nixfmt-rfc-style;
      }
    );
}
