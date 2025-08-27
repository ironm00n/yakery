{
  config,
  lib,
  my-utils,
  pkgs,
  pkgs-stable,
  ...
}:
let
  inherit (lib) types mkOption mkIf;
  inherit (my-utils) symlink;
  inherit (config.xdg) configHome;
  cfg = config.bundles.dev;
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
{
  options.bundles.dev = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable global dev stuff.";
    };

    langs = mkOption {
      type = types.bool;
      default = true;
      description = "Enable Langauges.";
    };

    jetbrains = mkOption {
      type = types.bool;
      default = true;
      description = "Enable Jetbrains products.";
    };

    tooling = mkOption {
      type = types.bool;
      default = true;
      description = "Enable dev tooling (IDEs, editors, etc).";
    };
  };

  config = mkIf cfg.enable {
    home.packages =
      with pkgs;
      lib.optionals cfg.langs [
        (python3.withPackages (used-python-pkgs))
        # yarn-berry
        nodejs_24
        corepack_24
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

        jdk
        (pkgs.lowPrio jdk11)
        (pkgs.lowPrio jdk17)

        typescript
        typescript-language-server
      ]
      ++ lib.optionals cfg.jetbrains [
        pkgs-stable.jetbrains.idea-ultimate
        pkgs-stable.jetbrains.datagrip
        pkgs-stable.jetbrains.webstorm
        pkgs-stable.jetbrains.pycharm-professional
        pkgs-stable.jetbrains.clion
      ]
      ++ lib.optionals cfg.tooling [
        code-cursor
        lazygit
      ];

    home.file = mkIf cfg.tooling {
      "${configHome}/lazygit/config.yml".source = symlink ./lazygit.yml;
    };
  };
}
