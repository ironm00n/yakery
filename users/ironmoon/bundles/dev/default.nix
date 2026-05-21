{
  config,
  lib,
  my-lib,
  my-utils,
  pkgs,
  pkgs-stable,
  ...
}:
let
  inherit (lib) mkEnableOption mkIf lowPrio;
  inherit (my-lib) mkDisableOption;
  inherit (my-utils) symlink;
  inherit (config.xdg) configHome;
  cfg = config.bundles.dev;
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
    ];

  inherit (pkgs) claude-code fetchurl;

  pinned-claude-code = claude-code.overrideAttrs (
    finalAttrs:
    let
      # https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases/latest
      # https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases/2.1.146/manifest.json
      version = "2.1.146";
      baseUrl = "https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases";
    in
    {
      inherit version;
      src = fetchurl {
        url = "${baseUrl}/${version}/linux-x64/claude";
        sha256 = "825d5301380f1f5f466c5268de25a062927be658938fc1d630cfa02c521b8185";
      };
    }
  );
  effective-claude-code = if cfg.pinned-claude then pinned-claude-code else claude-code;
in
{
  options.bundles.dev = {
    enable = mkEnableOption "global dev stuff";
    langs = mkDisableOption "languages";
    jetbrains = mkDisableOption "Jetbrains products";
    tooling = mkDisableOption "dev tooling (IDEs, editors, etc)";
    pinned-claude = mkEnableOption "pin claude-code version manually";
    other-llm = mkEnableOption "enable rarely used llm tooling";
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

        effective-claude-code
      ]
      ++ lib.optionals cfg.other-llm [
        code-cursor
        antigravity.fhs
        windsurf
      ];

    home.file = mkIf cfg.tooling {
      "${configHome}/lazygit/config.yml".source = symlink ./lazygit.yml;
    };
  };
}
