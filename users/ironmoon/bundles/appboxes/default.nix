{
  config,
  lib,
  pkgs,
  host,
  ...
}:
let
  inherit (lib)
    attrNames
    attrValues
    concatLists
    concatMap
    concatMapStrings
    concatStrings
    concatStringsSep
    escapeShellArg
    filter
    hasInfix
    listToAttrs
    mapAttrs'
    mapAttrsToList
    mkIf
    mkMerge
    mkOption
    nameValuePair
    optional
    optionalString
    types
    ;
  cfg = config.bundles.appboxes;

  sw = "/run/current-system/sw/bin";

  multiarch =
    {
      x86_64-linux = "x86_64-linux-gnu";
      aarch64-linux = "aarch64-linux-gnu";
    }
    .${pkgs.stdenv.hostPlatform.system}
      or (throw "appboxes: no Debian multiarch triplet for ${pkgs.stdenv.hostPlatform.system}");

  badNames = filter (n: builtins.match "[a-z][a-z0-9-]*" n == null) (
    attrNames cfg.boxes
    ++ concatMap (b: attrNames b.apps ++ attrNames b.aptRepos) (attrValues cfg.boxes)
  );

  keyring = repo: "/usr/share/keyrings/${repo}-archive-keyring.asc";

  verifiedKey =
    repo: r:
    pkgs.runCommand "${repo}-archive-keyring.asc" { nativeBuildInputs = [ pkgs.gnupg ]; } ''
      export GNUPGHOME=$(mktemp -d)
      fpr=$(gpg --batch --with-colons --show-keys ${r.key} | awk -F: '$1 == "fpr" { print $10; exit }')
      [ "$fpr" = ${r.fingerprint} ] || {
        echo "${repo}: primary fingerprint $fpr, expected ${r.fingerprint}" >&2
        exit 1
      }
      cp ${r.key} $out
    '';

  sourceLine =
    repo: r:
    let
      opts = optional (r.architectures != [ ]) "arch=${concatStringsSep "," r.architectures}" ++ [
        "signed-by=${keyring repo}"
      ];
    in
    "deb [${concatStringsSep " " opts}] ${r.uri} ${r.suite} ${concatStringsSep " " r.components}";

  # runs inside the box, hence Debian's sh rather than runtimeShell
  boxScript =
    n: text:
    pkgs.writeScript n ''
      #!/bin/sh
      set -eu
      ${text}
    '';

  mkBox =
    name: box:
    let
      container = "appbox-${name}";
      rel = "appboxes/${name}/.appbox";
      boxHome = box.home;
      appbox = "${boxHome}/.appbox";
      iconFile = a: "${appbox}/icons/${baseNameOf a.icon}";
      entry = app: "${container}-${app}";

      # the base image has no ca-certificates and distrobox-init's first apt-get
      # update precedes its dependency install, so https repos need this first
      preInit = boxScript "${container}-pre-init" (
        optionalString (box.aptRepos != { }) ''
          if [ ! -e /usr/sbin/update-ca-certificates ]; then
            apt-get update -q && apt-get install -y -q ca-certificates
          fi
        ''
        + concatStrings (
          mapAttrsToList (repo: r: ''
            install -Dm644 ${verifiedKey repo r} ${keyring repo}
            printf '%s\n' ${escapeShellArg (sourceLine repo r)} > /etc/apt/sources.list.d/${repo}.list
          '') box.aptRepos
        )
      );

      init = boxScript "${container}-init" (
        ''
          ln -sf ${appbox}/bin/xdg-open /usr/local/bin/xdg-open
        ''
        + optionalString host.nvidia ''
          install -d /usr/lib/${multiarch}/gbm
          ln -sf /run/opengl-driver/lib/gbm/nvidia-drm_gbm.so /usr/lib/${multiarch}/gbm/nvidia-drm_gbm.so
        ''
        + concatStrings (
          mapAttrsToList (
            _: a:
            optionalString (a.icon != null) ''
              if [ -e ${a.icon} ]; then
                install -m644 ${a.icon} ${iconFile a}
                chown --reference=${appbox}/icons ${iconFile a}
              fi
            ''
          ) box.apps
        )
      );

      unlabeled =
        concatStringsSep "\n" (
          [
            "[${container}]"
            "image=${box.image}"
            "home=${boxHome}"
            "entry=false"
          ]
          ++ map (p: "volume=${p}:${p}") ([ "/run/dbus/system_bus_socket" ] ++ box.hostPaths)
          ++ [
            "additional_packages=${concatStringsSep " " ([ "libglib2.0-bin" ] ++ box.packages)}"
            "additional_flags=--env GTK_USE_PORTAL=1"
            "additional_flags=--http-proxy=false"
          ]
          ++ optional host.nvidia "additional_flags=--device nvidia.com/gpu=all"
          ++ [
            "pre_init_hooks=${preInit}"
            "init_hooks=${init}"
          ]
        )
        + "\n";
      hash = builtins.hashString "sha256" unlabeled;
      manifest = pkgs.writeText "${container}.ini" (
        unlabeled + "additional_flags=--label appbox.manifest=${hash}\n"
      );

      wrapper = pkgs.writeShellApplication {
        name = "appbox-${name}";
        runtimeInputs = [
          pkgs.util-linux
          pkgs.coreutils
          pkgs.libnotify
        ];
        text = ''
          box=${container}
          want=${hash}
          export PATH=${sw}:$PATH:${boxHome}/.local/bin DBX_CONTAINER_MANAGER=podman
          ${concatStringsSep "\n" (mapAttrsToList (k: v: "export ${k}=${escapeShellArg v}") box.environment)}
          fail() {
            echo "appbox-${name}: $*" >&2
            [ -t 2 ] || notify-send -u critical "appbox-${name}" "$*"
            exit 1
          }
          # shellcheck disable=SC2329
          on_err() { fail "$BASH_COMMAND failed"; }
          trap on_err ERR
          note() {
            echo "appbox-${name}: $*" >&2
            [ -t 2 ] || notify-send "appbox-${name}" "$*"
          }
          # keep the container's rootfs (apt state) and re-apply the current definition on top of it
          rebase() {
            note "definition changed; rebasing $box on its current package state"
            local old img
            old=$(${sw}/podman inspect -f '{{.ImageName}}' "$box")
            img=localhost/$box:$(date +%s)
            ${sw}/podman start "$box" >/dev/null
            ${sw}/podman exec --user root "$box" rm -f /.containersetupdone
            ${sw}/podman commit --quiet "$box" "$img" >/dev/null
            ${sw}/distrobox rm --force "$box" >/dev/null
            sed "s|^image=.*|image=$img|" ${manifest} > "$lockdir/$box.ini"
            ${sw}/distrobox assemble create --file "$lockdir/$box.ini" --name "$box"
            case $old in localhost/$box:*) ${sw}/podman rmi "$old" >/dev/null ;; esac
          }
          recreate=0
          if [ "''${1-}" = --recreate ]; then
            recreate=1
            shift
          fi
          lockdir="''${XDG_RUNTIME_DIR:-/tmp/appbox-$(id -u)}"
          (umask 077 && mkdir -p "$lockdir")
          exec 9>"$lockdir/$box.lock"
          flock 9
          if [ "$recreate" = 1 ] && ${sw}/podman container exists "$box"; then
            ${sw}/distrobox rm --force "$box"
          fi
          if ${sw}/podman container exists "$box"; then
            have=$(${sw}/podman inspect -f '{{index .Config.Labels "appbox.manifest"}}' "$box")
            [ -n "$have" ] || fail "$box exists but was not created by appbox-${name}; remove it or rename the box"
            [ "$have" = "$want" ] || rebase
          else
            ${sw}/distrobox assemble create --file ${manifest} --name "$box"
          fi
          have=$(${sw}/podman inspect -f '{{index .Config.Labels "appbox.manifest"}}' "$box")
          [ "$have" = "$want" ] || fail "$box still carries manifest $have after create/rebase; 'appbox-${name} --recreate' starts over"
          exec 9>&-
          trap - ERR
          cmd=''${1-shell}
          # distrobox maps the cwd to /run/host/...; keep it where the host and the box agree on the path
          if [ $# -gt 0 ]; then
            case $PWD in
              "$HOME" | "$HOME"/* | /tmp | /tmp/* | /nix | /nix/*${
                concatMapStrings (p: " | ${p} | ${p}/*") box.hostPaths
              }) set -- env --chdir="$PWD" PWD="$PWD" "$@" ;;
            esac
          fi
          set +e
          ${sw}/distrobox-enter -n "$box" -- "$@"
          rc=$?
          case $rc in
            126 | 127) fail "cannot run $cmd in $box (exit $rc); is it installed?" ;;
          esac
          exit "$rc"
        '';
      };

      schemeDefaults = listToAttrs (
        concatLists (
          mapAttrsToList (
            app: a: map (s: nameValuePair "x-scheme-handler/${s}" "${entry app}.desktop") a.urlSchemes
          ) box.apps
        )
      );
    in
    {
      inherit wrapper;
      configFile."distrobox/${name}.ini".source = manifest;
      dataFile = {
        "${rel}/bin/xdg-open" = {
          executable = true;
          text = ''
            #!/bin/sh
            exec gdbus call --session --dest org.freedesktop.portal.Desktop \
              --object-path /org/freedesktop/portal/desktop \
              --method org.freedesktop.portal.OpenURI.OpenURI "" "$1" "{}" >/dev/null
          '';
        };
        "${rel}/icons/.keep".text = "";
      };
      desktopEntries = mapAttrs' (
        app: a:
        nameValuePair (entry app) {
          inherit (a) name categories;
          exec = "appbox-${name} ${a.exec}";
          icon = if a.icon == null then null else iconFile a;
          mimeType = a.mimeType ++ map (s: "x-scheme-handler/${s}") a.urlSchemes;
          terminal = false;
        }
      ) box.apps;
      mimeApps = mkIf (!config.bundles.mime-apps.enable && schemeDefaults != { }) {
        enable = true;
        defaultApplications = schemeDefaults;
      };
      assertions = mapAttrsToList (mime: desktop: {
        assertion =
          !config.bundles.mime-apps.enable
          || hasInfix "${mime}=${desktop}" (builtins.readFile ../mime-apps/mimeapps.list);
        message = "bundles.appboxes: bundles.mime-apps owns mimeapps.list; add `${mime}=${desktop}` to its [Default Applications]";
      }) schemeDefaults;
    };
  boxes = mapAttrsToList mkBox cfg.boxes;
in
{
  imports = [ ./claude.nix ];

  options.bundles.appboxes.boxes = mkOption {
    default = { };
    description = "Debian containers, one per vendor app; `appbox-<name> <cmd>` creates the box on first use.";
    type = types.attrsOf (
      types.submodule (
        { name, ... }:
        {
          options = {
            image = mkOption {
              type = types.str;
              default = "docker.io/library/debian:13";
            };
            home = mkOption {
              type = types.str;
              default = "${config.xdg.dataHome}/appboxes/${name}";
              description = "The box's HOME on the host; the real home is mounted alongside it.";
            };
            environment = mkOption {
              type = types.attrsOf types.str;
              default = { };
              description = "Exported by the launcher for everything entering the box; HOME stays the box home.";
            };
            packages = mkOption {
              type = types.listOf types.str;
              default = [ ];
              description = "Installed at first setup only; changing them needs `--recreate`.";
            };
            hostPaths = mkOption {
              type = types.listOf (types.strMatching "^/[^:]+$");
              default = [ ];
              description = "Host directories visible at the same path inside the box (the home always is).";
            };
            aptRepos = mkOption {
              default = { };
              type = types.attrsOf (
                types.submodule {
                  options = {
                    uri = mkOption { type = types.str; };
                    suite = mkOption { type = types.str; };
                    components = mkOption {
                      type = types.listOf types.str;
                      default = [ "main" ];
                    };
                    architectures = mkOption {
                      type = types.listOf types.str;
                      default = [ ];
                    };
                    key = mkOption { type = types.path; };
                    fingerprint = mkOption {
                      type = types.strMatching "[0-9A-F]{40}";
                      description = "Primary key fingerprint; `key` is checked against it at build time.";
                    };
                  };
                }
              );
            };
            apps = mkOption {
              default = { };
              type = types.attrsOf (
                types.submodule {
                  options = {
                    name = mkOption { type = types.str; };
                    exec = mkOption { type = types.str; };
                    icon = mkOption {
                      type = types.nullOr types.str;
                      default = null;
                      description = "Icon path inside the box; copied out once the app is installed.";
                    };
                    urlSchemes = mkOption {
                      type = types.listOf types.str;
                      default = [ ];
                      description = "URL schemes this app becomes the default handler for.";
                    };
                    mimeType = mkOption {
                      type = types.listOf types.str;
                      default = [ ];
                    };
                    categories = mkOption {
                      type = types.listOf types.str;
                      default = [ ];
                    };
                  };
                }
              );
            };
          };
        }
      )
    );
  };

  # top-level keys stay static; only their values depend on cfg.boxes
  config = mkIf host.appboxes {
    assertions = [
      {
        assertion = badNames == [ ];
        message = "bundles.appboxes: names must match [a-z][a-z0-9-]*: ${concatStringsSep " " badNames}";
      }
    ]
    ++ concatMap (b: b.assertions) boxes;
    home.packages = map (b: b.wrapper) boxes;
    xdg.configFile = mkMerge (map (b: b.configFile) boxes);
    xdg.dataFile = mkMerge (map (b: b.dataFile) boxes);
    xdg.desktopEntries = mkMerge (map (b: b.desktopEntries) boxes);
    xdg.mimeApps = mkMerge (map (b: b.mimeApps) boxes);
  };
}
