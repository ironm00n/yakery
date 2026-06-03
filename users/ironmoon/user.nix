{
  pkgs,
  pkgs-stable,
  config,
  ...
}:
let
  creative = !config.host.lightweight;
  utils = true;
  productivity = true;
  non-essential = !config.host.lightweight;

  # Run Zoom in an app-us.zoom.Zoom-* systemd scope (even when running from terminal) so the portal
  # reports that app_id, which `force_linear_apps` matches.
  zoom-scoped =
    let
      zoom = pkgs.zoom-us.override {
        hyprlandXdgDesktopPortalSupport = config.host.hyprland;
        plasma6XdgDesktopPortalSupport = config.host.kde;
      };
    in
    pkgs.symlinkJoin {
      name = "zoom-us-app-scoped";
      paths = [
        (pkgs.writeShellScriptBin "zoom" ''
          exec ${pkgs.systemd}/bin/systemd-run --user --scope --unit="app-us.zoom.Zoom-$RANDOM" ${zoom}/bin/zoom "$@"
        '')
        zoom
      ];
    };
in
{
  # user account.
  # set a password with `passwd`
  users.users.ironmoon = {
    isNormalUser = true;
    description = "ironmoon";
    extraGroups = [
      "wheel"
      "adm"
      "input"
      "dialout"
      "networkmanager"
      "libvirtd"
      "docker"
      "kvm"
      "wireshark"
    ];
    shell = pkgs.zsh;

    packages =
      with pkgs;
      [ ]
      ++ lib.optionals config.host.hyprland [
        # HACK: see fix kde-colorscheme service
        kdePackages.kservice
        kdePackages.dolphin
        kdePackages.kdegraphics-thumbnailers # previews
        kdePackages.konsole
        kdePackages.ark

        # still want breeze
        kdePackages.breeze
        kdePackages.breeze-gtk
        kdePackages.breeze-icons
        kdePackages.qt6ct

        networkmanagerapplet
        nwg-displays
        wev
      ]
      ++ lib.optionals config.host.kde [
        kdePackages.plasma-desktop

        kdePackages.kcolorchooser
        kdePackages.plasma-firewall
        kdePackages.kde-cli-tools
      ]
      ++ lib.optionals creative [
        pkgs-stable.kdePackages.kdenlive
        gimp3
        inkscape
        krita
        pkgs-stable.blender
        obs-studio
        audacity
        pkgs-stable.aseprite # isn't cached and constant rebuilds are annoying
        musescore
      ]
      ++ lib.optionals productivity [
        thunderbird
        birdtray
        libreoffice-qt
        obsidian
        xournalpp
        zotero
        zoom-scoped
        slack
        zulip
        pdfpc
      ]
      ++ [
        # TODO: https://github.com/NixOS/nixpkgs/issues/371479
        # TODO: https://github.com/nix-community/home-manager/issues/5559
        # bitwarden-desktop

        playerctl

        comma
        nix-index

        home-manager
        nix-output-monitor

        shellcheck
        hunspell
        hunspellDicts.en_US

        (pkgs.callPackage ../../packages/hytale { })
      ]
      ++ lib.optionals utils [
        # https://github.com/ibraheemdev/modern-unix
        mcfly
        fzf
        broot
        duf
        dust
        bat
        bottom
        procs
        doggo
        glances
        gtop
        jq
        lf

        ripgrep-all
        pkg-config
        pandoc

        poppler-utils
      ]
      ++ lib.optionals non-essential [
        (wordlists.override {
          lists = with pkgs; [
            rockyou
            seclists
          ];
        })
        qbittorrent

        (fontforge.override {
          withGUI = true;
        })

        (pkgs-stable.calibre.override {
          unrarSupport = true; # .cbr, .cbz
        })
        epubcheck

        prismlauncher
        minecraft-server

        bytecode-viewer
        pkgs-stable.avogadro2
        pkgs-stable.openbabel
      ]
      ++ [
        vlc

        spotify

        evil-helix

        # HACK: see fix kde-colorscheme service
        kdePackages.kate
        kdePackages.kcalc
        kdePackages.ksystemlog
        kdePackages.ktimer
        kdePackages.kalarm
        kdePackages.kweather

        google-chrome
        # firefox enabled with home-manager
        firefox-devedition
        tor-browser

        element-desktop
        signal-desktop

        frp # fast reverse proxy
      ]
      ++ config.host.additional-user-pkgs;
  };
}
