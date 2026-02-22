{
  config,
  lib,
  my-utils,
  pkgs,
  pkgs-stable,
  ...
}:
let
  inherit (lib)
    types
    mkOption
    mkIf
    lowPrio
    ;
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
        nixfmt
        texlive.combined.scheme-full
        treefmt

        jdk
        (lowPrio jdk11)
        (lowPrio jdk17)

        typescript
        typescript-language-server
      ]
      ++ lib.optionals cfg.jetbrains (
        with pkgs-stable.jetbrains;
        [
          idea
          datagrip
          webstorm
          pycharm
          clion
        ]
      )
      ++ lib.optionals cfg.tooling [
        direnv
        nix-direnv
        arduino-ide
        lazygit

        claude-code
        code-cursor
        (antigravity.overrideAttrs (
          old:
          let
            version = "1.18.4";
            vscodeVersion = "1.107.0";
            url = "https://edgedl.me.gvt1.com/edgedl/release2/j0qc3/antigravity/stable/1.18.4-5780041996042240/linux-x64/Antigravity.tar.gz";
            sha256 = "f97d790d1fb74e8ccb9ddb6af301a2b60391aed22f633f1a2baf86862aa65826";
          in
          {
            pname = "${old.pname}-patched";
            inherit version vscodeVersion;
            src = fetchurl { inherit url sha256; };
          }
        )).fhs
        windsurf
      ];

    home.file = mkIf cfg.tooling {
      "${configHome}/lazygit/config.yml".source = symlink ./lazygit.yml;
    };
  };
}
