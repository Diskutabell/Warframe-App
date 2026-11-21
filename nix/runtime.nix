# What warframeproject needs at build and run time, shared by shell.nix and default.nix.
{ pkgs }:
rec {
  # Linked libraries.
  libraries = with pkgs; [
    gtk3
    webkitgtk_4_1
    libsoup_3
    glib-networking   # TLS for the item art the UI loads from warframe.market and warframestat
    gtk-layer-shell   # the reward/riven overlays on Wayland compositors (Hyprland, Sway, KDE)
  ];

  # Programs the app runs. Hyprland and Sway are spoken to over their own sockets (src/ipc.rs),
  # so their command-line tools are not needed.
  tools = with pkgs; [
    (tesseract.override { enableLanguages = [ "eng" ]; })
    grim       # screenshots when the compositor cannot be copied from directly (src/ocr/screencopy.rs)
    libnotify  # notify-send, for the optional desktop notification
  ];

  # Shell form of what wrapGAppsHook3 sets up for the packaged build: TLS for WebKit, and the
  # GSettings schemas GTK's file chooser needs (the UI imports inventory files through it).
  environment = ''
    export GIO_EXTRA_MODULES="${pkgs.glib-networking}/lib/gio/modules''${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"
    export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
  '';
}
