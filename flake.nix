{
  # The asciimaker CLI as a Nix package, so scripts can pin an exact commit:
  #   nix build github:jpossaz/ascii-maker-rs/<rev>   ->  ./result/bin/asciimaker
  #   nix run github:jpossaz/ascii-maker-rs -- -i image.png -w 100 -c
  description = "asciimaker: image to ASCII art";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/1c89083e451e891152365b799595cf7790a43325";

  outputs = { self, nixpkgs }:
    let pkgs = nixpkgs.legacyPackages.x86_64-linux;
    in {
      packages.x86_64-linux.default = pkgs.rustPlatform.buildRustPackage {
        pname = "asciimaker";
        version = "0.2.0";
        src = pkgs.lib.cleanSource ./.;
        cargoLock.lockFile = ./Cargo.lock;
      };
    };
}
