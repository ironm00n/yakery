{
  writeShellApplication,
  coreutils,
  findutils,
  winetricks,
  wine-adobe,
}:

writeShellApplication {
  name = "photoshop";
  runtimeInputs = [
    wine-adobe
    winetricks
    coreutils
    findutils
  ];
  text = ''
    prefix="''${PHOTOSHOP_PREFIX:-$HOME/.local/share/photoshop}"
    export WINEPREFIX="$prefix"
    export WINEARCH="win64"
    export WINE="${wine-adobe}/bin/wine"
    export WINEDLLOVERRIDES="''${WINEDLLOVERRIDES:-winemenubuilder.exe=d}"
    export WINEDEBUG="''${WINEDEBUG:-fixme-all}"

    usage() {
      echo "usage: photoshop {init | installer <exe> | run | cfg | tricks <verbs...> | wine <args...> | prefix}" >&2
    }

    cmd="''${1:-run}"
    if [ "$#" -gt 0 ]; then shift; fi

    case "$cmd" in
      init)
        mkdir -p "$prefix"
        wineboot --init
        winetricks -q win10 corefonts fontsmooth=rgb
        echo "prefix ready: $prefix"
        echo "next: photoshop installer /path/to/Adobe-Creative-Cloud-installer.exe"
        ;;
      installer | install)
        if [ "$#" -lt 1 ]; then usage; exit 2; fi
        exec wine "$@"
        ;;
      run)
        exe="$(find "$prefix/drive_c" -path '*Adobe Photoshop*/Photoshop.exe' -print -quit 2>/dev/null || true)"
        if [ -z "$exe" ]; then
          echo "Photoshop.exe not found under $prefix; install it first (photoshop installer <exe>)" >&2
          exit 1
        fi
        exec wine "$exe" "$@"
        ;;
      cfg) exec winecfg "$@" ;;
      tricks) exec winetricks "$@" ;;
      wine) exec wine "$@" ;;
      prefix) echo "$prefix" ;;
      *) usage; exit 2 ;;
    esac
  '';
}
