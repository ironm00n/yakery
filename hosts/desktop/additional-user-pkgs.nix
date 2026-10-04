{ pkgs }:
with pkgs;
[
  papermc
  android-studio
  # desktop has no Wayland session; Claude Code's image paste shells out to wl-paste, so read fw13's
  (writeShellApplication {
    name = "wl-paste";
    runtimeInputs = [ openssh ];
    text = ''
      if [ -n "''${WAYLAND_DISPLAY:-}" ]; then exec ${wl-clipboard}/bin/wl-paste "$@"; fi
      exec ssh -o BatchMode=yes -o ConnectTimeout=3 "''${CLIP_HOST:-fw13}" \
        "export XDG_RUNTIME_DIR=/run/user/\$(id -u); export WAYLAND_DISPLAY=\$(ls \$XDG_RUNTIME_DIR | grep -m1 '^wayland-[0-9]*\$'); exec wl-paste ''${*@Q}"
    '';
  })
]
