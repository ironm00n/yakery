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
in
{
  # user account.
  # set a password with `passwd`
  users.users.ironmoon = {
    isNormalUser = true;
    description = "ironmoon";
    extraGroups = [
      "networkmanager"
      "wheel"
      "libvirtd"
      "docker"
      "kvm"
      "dialout"
      "input"
      "wireshark"
    ];
    shell = pkgs.zsh;

    packages =
      with pkgs;
      [ ]
      ++ lib.optionals config.host.hyprland [
        kdePackages.kservice
        kdePackages.dolphin
        kdePackages.kdegraphics-thumbnailers # previews
        kdePackages.konsole
        kdePackages.ark

        # still want breeze
        kdePackages.breeze
        kdePackages.breeze-gtk
        kdePackages.breeze-icons

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
        kdePackages.kdenlive
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
        zoom-us
        slack
      ]
      ++ [
        # TODO: https://github.com/NixOS/nixpkgs/issues/371479
        # TODO: https://github.com/NixOS/nixpkgs/pull/374068
        # TODO: https://github.com/NixOS/nixpkgs/issues/347350
        bitwarden-desktop

        playerctl

        nix-index
        home-manager
        nix-output-monitor

        shellcheck
        hunspell
        hunspellDicts.en_US
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

        ripgrep-all
        pkg-config
        pandoc
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

        (calibre.override {
          unrarSupport = true; # .cbr, .cbz
        })
        epubcheck

        prismlauncher
        minecraft-server

        bytecode-viewer
        # FIXME: broken
        # avogadro2
        openbabel
      ]
      ++ [
        vlc

        spotify

        evil-helix

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
