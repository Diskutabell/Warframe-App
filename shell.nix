# Development shell: the Rust toolchain, the GTK/WebKit libraries the app links against, and the
# tools it runs (tesseract, grim, notify-send). `./run.sh` uses it automatically on NixOS.
{ pkgs ? import <nixpkgs> { } }:
let
  runtime = import ./nix/runtime.nix { inherit pkgs; };
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [ cargo rustc rustfmt clippy pkg-config ];
  buildInputs = runtime.libraries;
  packages = runtime.tools;
  shellHook = runtime.environment;
}
