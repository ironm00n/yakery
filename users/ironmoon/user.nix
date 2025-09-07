{
  pkgs,
  pkgs-stable,
  config,
  ...
}:
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

        # still want breeze
        kdePackages.breeze
        kdePackages.breeze-gtk
        kdePackages.breeze-icons
      ]
      ++ lib.optionals config.host.kde [
        kdePackages.plasma-desktop

        kdePackages.kcolorchooser
        kdePackages.plasma-firewall
        kdePackages.kde-cli-tools
      ]
      ++ [
        nix-index
        home-manager
        nix-output-monitor

        networkmanagerapplet
        nwg-displays
        wev

        kdePackages.kate
        kdePackages.kdenlive
        kdePackages.kcalc
        kdePackages.ksystemlog
        kdePackages.ktimer
        kdePackages.kalarm
        kdePackages.kweather

        google-chrome
        # firefox enabled with home-manager
        firefox-devedition
        tor-browser-bundle-bin
        element-desktop

        thunderbird
        birdtray
        libreoffice-qt
        hunspell
        hunspellDicts.en_US
        gimp3
        inkscape
        krita
        pkgs-stable.blender
        obs-studio
        vlc
        audacity
        zoom-us
        pkg-config
        obsidian
        krita
        pkgs-stable.aseprite # isn't cached and constant rebuilds are annoying
        xournalpp
        zotero

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

        slack
        signal-desktop

        spotify

        pandoc

        (wordlists.override {
          lists = with pkgs; [
            rockyou
            seclists
          ];
        })

        evil-helix

        qbittorrent

        frp # fast reverse proxy
        ripgrep-all

        (fontforge.override {
          withGUI = true;
        })

        (calibre.override {
          # broken again...
          # unrarSupport = true; # .cbr, .cbz
        })
        epubcheck

        # (minecraft.overrideAttrs (oldAttrs: {
        #   meta = oldAttrs.meta // {
        #     broken = false;
        #   };
        # }))
        prismlauncher
        minecraft-server

        bytecode-viewer
        avogadro2
        openbabel
        shellcheck

        # TODO: https://github.com/NixOS/nixpkgs/issues/371479
        # TODO: https://github.com/NixOS/nixpkgs/pull/374068
        # TODO: https://github.com/NixOS/nixpkgs/issues/347350
        bitwarden-desktop

        playerctl
      ]
      ++ config.host.additional-user-pkgs;
  };
}
