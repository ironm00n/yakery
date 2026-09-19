# The agents nobody packages: claude-desktop ships as a .deb, muse as a vendor installer, dsh as
# an npm package. Nothing here runs on its own — type `hackathon-apps` in the guest when you want
# them. What makes the foreign binaries run at all is nix-ld and envfs in the guest, not anything
# in this file. (The Codex app is a real package via codex-desktop-linux, so it isn't here.)
{ pkgs }:
pkgs.writeShellApplication {
  name = "hackathon-apps";
  runtimeInputs = with pkgs; [
    coreutils
    curl
    dpkg
    gawk
    gnused
    nodejs_22
  ];
  text = ''
    data_home="''${XDG_DATA_HOME:-$HOME/.local/share}"
    apps_root="$data_home/deb-apps"
    bin_dir="$HOME/.local/bin"

    # A .deb's paths are absolute, so everything that points at /usr or /opt has to be bent
    # back into the unpack directory: argv[0] symlinks, and the desktop entry's own keys.
    install_deb() {
      local name="$1" url="$2" dir tmp entry base target
      local -a rewrites
      dir="$apps_root/$name"
      tmp="$(mktemp -d)"
      echo "==> $name"
      curl -fL --progress-bar -o "$tmp/app.deb" "$url"
      rm -rf "$dir"
      mkdir -p "$dir" "$bin_dir" "$data_home/applications" "$data_home/icons"
      dpkg-deb -x "$tmp/app.deb" "$dir"
      rm -rf "$tmp"

      for entry in "$dir"/usr/bin/*; do
        [ -e "$entry" ] || [ -L "$entry" ] || continue
        base="$(basename "$entry")"
        target="$entry"
        if [ -L "$entry" ]; then
          target="$(readlink "$entry")"
          case "$target" in
            /*) target="$dir$target" ;;
            *) target="$(dirname "$entry")/$target" ;;
          esac
        fi
        ln -sfn "$target" "$bin_dir/$base"
        # A bare command in Exec= resolves only through the launcher's PATH; name the binary.
        rewrites+=(-e "s#^(Exec|TryExec)=$base( |\$)#\1=$target\2#")
      done

      for entry in "$dir"/usr/share/applications/*.desktop; do
        [ -e "$entry" ] || continue
        sed -E -e "s#^(Exec|TryExec|Icon)=/#\1=$dir/#" "''${rewrites[@]}" "$entry" \
          > "$data_home/applications/$(basename "$entry")"
      done

      if [ -d "$dir/usr/share/icons" ]; then
        cp -rn "$dir/usr/share/icons/." "$data_home/icons/" || true
      fi
    }

    install_claude_desktop() {
      local base="https://downloads.claude.ai/claude-desktop/apt/stable" file
      file="$(curl -fsSL "$base/dists/stable/main/binary-amd64/Packages" \
        | awk '/^Filename:/ { print $2 }' | sort -V | tail -1)"
      if [ -z "$file" ]; then
        echo "claude-desktop: no amd64 package in the apt index" >&2
        return 1
      fi
      install_deb claude-desktop "$base/$file"
    }

    install_dsh() {
      echo "==> dsh"
      npm_config_prefix="$HOME/.local" npm install -g @deepseek-ai/dsh
    }

    # The installer only places a launcher; the first run is what fetches the 300MB binary, so
    # do that here rather than the first time you actually want it.
    install_muse() {
      echo "==> muse"
      curl -fsSL https://dev.meta.ai/install.sh | bash
      "$bin_dir/muse" --version || true
    }

    usage() {
      echo "usage: hackathon-apps [claude-desktop|dsh|muse ...]"
    }

    targets=("$@")
    if [ ''${#targets[@]} -eq 0 ]; then
      targets=(claude-desktop dsh muse)
    fi
    for target in "''${targets[@]}"; do
      case "$target" in
        claude-desktop) install_claude_desktop ;;
        dsh) install_dsh ;;
        muse) install_muse ;;
        -h | --help) usage && exit 0 ;;
        *) usage >&2 && exit 1 ;;
      esac
    done
  '';
}
