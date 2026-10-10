# warframeproject as a Nix package.
#
#   nix-build                     # then ./result/bin/warframeproject
#   nix-env -f . -i               # install into your profile (menu entry included)
#
# or in a NixOS configuration: environment.systemPackages = [ (pkgs.callPackage /path/to/checkout { }) ];
{ pkgs ? import <nixpkgs> { } }:
let
  runtime = import ./nix/runtime.nix { inherit pkgs; };
  cargo = builtins.fromTOML (builtins.readFile ./Cargo.toml);
in
pkgs.rustPlatform.buildRustPackage {
  pname = "warframeproject";
  inherit (cargo.package) version;

  # Only what the build reads: not build output or local results.
  src = pkgs.lib.cleanSourceWith {
    src = pkgs.lib.cleanSource ./.;
    filter = path: _type: !(builtins.elem (baseNameOf path) [ "target" "result" ]);
  };
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook3 ];
  buildInputs = runtime.libraries ++ [ pkgs.gsettings-desktop-schemas ];

  # wrapGAppsHook3 sets up GSettings schemas and GIO modules (TLS for the item art); the tools the
  # app runs go on its PATH. Compositor tools (hyprctl, swaymsg) come from the running system.
  preFixup = ''
    gappsWrapperArgs+=(--prefix PATH : ${pkgs.lib.makeBinPath runtime.tools})
  '';

  postInstall = ''
    install -Dm644 warframeproject.desktop $out/share/applications/warframeproject.desktop
    substituteInPlace $out/share/applications/warframeproject.desktop \
      --replace-fail "Exec=/path/to/warframeproject/run.sh" "Exec=$out/bin/warframeproject"
  '';

  meta = {
    description = cargo.package.description;
    mainProgram = "warframeproject";
    platforms = pkgs.lib.platforms.linux;
  };
}
