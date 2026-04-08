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

        (claude-code-bin.overrideAttrs (
          finalAttrs:
          let
            version = "2.1.92";
            baseUrl = "https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases";
          in
          {
            inherit version;
            src = fetchurl {
              url = "${baseUrl}/${version}/linux-x64/claude";
              sha256 = "e22324514967ff2d5e9f91f0ee37e4675bf8b6dfec27fafb19cb25cc5b23fcaf";
            };
          }
        ))
        code-cursor
        antigravity.fhs
        windsurf
      ];

    home.file = mkIf cfg.tooling {
      "${configHome}/lazygit/config.yml".source = symlink ./lazygit.yml;
    };
  };
}
