# warframeproject as a Nix package.
#
#   nix-build                             # then ./result/bin/warframeproject
#   nix-env -f . -i                       # install into your profile (menu entry included)
#   nix-env -f . -i --arg memscan false   # the same, built without the memory scan
#
# or in a NixOS configuration: environment.systemPackages = [ (pkgs.callPackage /path/to/checkout { }) ];
# with `{ memscan = false; }` in place of `{ }` for the build without the memory scan.
{
  pkgs ? import <nixpkgs> { },
  # The optional memory scan, off until it is switched on in Settings. `false` builds the app
  # without it: no code that opens or reads another process is compiled in.
  memscan ? true,
}:
let
  runtime = import ./nix/runtime.nix { inherit pkgs; };
  cargo = builtins.fromTOML (builtins.readFile ./Cargo.toml);
  # Neither build output nor installed dependencies go into a build.
  clean = src: pkgs.lib.cleanSourceWith {
    src = pkgs.lib.cleanSource src;
    filter = path: _type: !(builtins.elem (baseNameOf path) [ "target" "result" "node_modules" "dist" ]);
  };
  # The UI (web/, Svelte), built once and embedded by the engine (src/api/mod.rs).
  ui = pkgs.buildNpmPackage {
    pname = "warframeproject-ui";
    inherit (cargo.package) version;
    src = clean ./web;
    # After changing web/package-lock.json: nix-shell -p prefetch-npm-deps --run 'prefetch-npm-deps web/package-lock.json'
    npmDepsHash = "sha256-sAH8IgpxDY1nzplVTvBH7N9q346qD3Bo5jZ1vRr5Ymw=";
    installPhase = ''
      runHook preInstall
      cp -r dist $out
      runHook postInstall
    '';
  };
in
pkgs.rustPlatform.buildRustPackage {
  pname = "warframeproject";
  inherit (cargo.package) version;

  # Only what the build reads: not build output or local results.
  src = clean ./.;
  cargoLock.lockFile = ./Cargo.lock;
  # `memscan` is the crate's only default feature, so leaving the defaults out leaves out just
  # that, in the tests too.
  buildNoDefaultFeatures = !memscan;

  preBuild = ''
    cp -r ${ui} web/dist
    chmod -R u+w web/dist
  '';

  nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook3 ];
  # sqlite and zstd: their crates link the system copies when the build environment offers them.
  buildInputs = runtime.libraries ++ [ pkgs.gsettings-desktop-schemas pkgs.sqlite pkgs.zstd ];

  # wrapGAppsHook3 sets up GSettings schemas and GIO modules (TLS for the item art); the tools the
  # app runs go on its PATH. Compositor tools (hyprctl, swaymsg) come from the running system.
  preFixup = ''
    gappsWrapperArgs+=(--prefix PATH : ${pkgs.lib.makeBinPath runtime.tools})
  '';

  # The menu entry names the command, not its store path: docks and launchers copy the Exec line
  # when the app is pinned, and a store path would keep starting the old build after every update.
  postInstall = ''
    install -Dm644 warframeproject.desktop $out/share/applications/warframeproject.desktop
    substituteInPlace $out/share/applications/warframeproject.desktop \
      --replace-fail "Exec=/path/to/warframeproject/run.sh" "Exec=warframeproject"
  '';

  meta = {
    description = cargo.package.description;
    mainProgram = "warframeproject";
    license = pkgs.lib.licenses.mit;
    platforms = pkgs.lib.platforms.linux;
  };
}
