{
  config,
  lib,
  my-lib,
  my-utils,
  pkgs,
  pkgs-stable,
  pkgs-master,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf lowPrio;
  inherit (lib) mkMerge;
  inherit (my-lib) mkDisableOption;
  inherit (my-utils) symlink;
  cfg = config.bundles.dev;
in
{
  imports = [ ./claude ];

  options.bundles.dev = {
    enable = mkEnableOption "global dev stuff";
    langs = mkDisableOption "languages";
    jetbrains = mkDisableOption "Jetbrains products";
    tooling = mkDisableOption "dev tooling (IDEs, editors, etc)";
    other-llm = mkEnableOption "enable rarely used llm tooling";
    codex-desktop = mkEnableOption "Codex desktop app (ilysenko/codex-desktop-linux, built from the official .deb)";
  };

  config = mkIf cfg.enable (mkMerge [
    { home.packages = [ (pkgs.callPackage ./comma-noninteractive.nix { }) ]; }
    (mkIf cfg.codex-desktop { programs.codexDesktopLinux.enable = true; })
    (mkIf cfg.langs (
      let
        used-python-pkgs =
          python-pkgs: with python-pkgs; [
            z3-solver

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

            rich
          ];
      in
      {
        home.packages = with pkgs; [
          (python3.withPackages (used-python-pkgs))
          (runCommand "nodejs-wrapped"
            {
              nativeBuildInputs = [ pkgs.makeWrapper ];
            }
            ''
              mkdir -p $out/bin
              for bin in ${pkgs.nodejs_24}/bin/*; do
                makeWrapper "$bin" "$out/bin/$(basename "$bin")" \
                  --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath [ pkgs.libuuid ]}"
              done
            ''
          )
          ghc
          racket
          nil
          nixfmt
          texlive.combined.scheme-full
          treefmt

          jdk
          (lowPrio jdk11)
          (lowPrio jdk17)

          typescript
          typescript-language-server
        ];
      }
    ))
    (mkIf cfg.jetbrains {
      home.packages = with pkgs-stable.jetbrains; [
        idea
        datagrip
        webstorm
        pycharm
        clion
      ];
    })
    (mkIf cfg.tooling {
      home.packages = with pkgs; [
        direnv
        nix-direnv
        arduino-ide
        lazygit
        jujutsu
        lazyjj
        jjui
        tuicr
        gh-dash
      ];

      xdg.configFile."lazygit/config.yml".source = symlink ./lazygit.yml;
      xdg.configFile."jj/config.toml".source = symlink ./jj.toml;
      xdg.configFile."jjui/config.toml".source = symlink ./jjui.toml;
    })
    (mkIf cfg.other-llm {
      home.packages = with pkgs; [
        # code-cursor
        antigravity-cli
        cursor-cli
        # windsurf
      ];
    })
    {
      home.packages = with pkgs-master; [
        codex
      ];
    }
  ]);
}
