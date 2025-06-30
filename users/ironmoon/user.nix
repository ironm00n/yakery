{
  pkgs,
  pkgs-stable,
  config,
  ...
}:
{
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
    let
      used-python-pkgs =
        python-pkgs: with python-pkgs; [
          pandas
          matplotlib
          flask
          flask-session
          # requests

          annotated-types
          anyio
          certifi
          charset-normalizer
          distro
          h11
          httpcore
          httpx
          idna
          openai
          pydantic
          pydantic-core
          regex
          requests
          sniffio
          tiktoken
          tqdm
          typing-extensions
          urllib3
          python-dotenv

          ipykernel
          grip
          sympy
          cryptography
          bitarray
          gmpy2
          beautifulsoup4
          pyasn1

          setuptools

          pwntools

          pytest
          pynvim
          # jd-gui # removed
        ];
    in
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
      kdePackages.kwallet-pam
      kdePackages.plasma-firewall
      libsForQt5.kde-cli-tools
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
      ladybird

      thunderbird
      birdtray
      libreoffice-qt
      hunspell
      hunspellDicts.en_US
      gimp
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
      lazygit
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

      (python3.withPackages (used-python-pkgs))
      # yarn-berry
      nodejs_22
      corepack_22
      pm2
      ocaml
      ocamlPackages.utop
      dune_3
      opam
      ghc
      racket
      nil
      nixfmt-rfc-style
      texlive.combined.scheme-full
      arduino-ide
      direnv
      nix-direnv
      treefmt

      pandoc

      (wordlists.override {
        lists = with pkgs; [
          rockyou
          seclists
        ];
      })

      postman
      ghidra
      burpsuite
      metasploit
      ida-free

      pkgs-stable.jetbrains.idea-ultimate
      pkgs-stable.jetbrains.datagrip
      pkgs-stable.jetbrains.webstorm
      pkgs-stable.jetbrains.pycharm-professional
      pkgs-stable.jetbrains.clion

      evil-helix
      code-cursor

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
      jdk
      jdk11
      jdk17
      avogadro2
      openbabel
      shellcheck

      # TODO: https://github.com/NixOS/nixpkgs/issues/371479
      # TODO: https://github.com/NixOS/nixpkgs/pull/374068
      # TODO: https://github.com/NixOS/nixpkgs/issues/347350
      bitwarden-desktop

      typescript
      typescript-language-server
    ]
    ++ config.host.additional-user-pkgs;
}
