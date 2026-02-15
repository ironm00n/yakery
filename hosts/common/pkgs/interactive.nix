{ pkgs }:

with pkgs;
[
  comma

  dmidecode
  os-prober

  exiftool

  usbmuxd

  wl-clipboard
  git-filter-repo

  kitty
  emacs-gtk
  # use the built-in settings sync
  ((vscode.overrideAttrs (
    old:
    let
      version = "1.109.2";
      plat = "linux-x64";
      hash = "sha256-ST5i8gvNtAaBbmcpcg9GJipr8e5d0A0qbdG1P9QViek=";
    in
    {
      inherit version;
      src = fetchurl {
        name = "VSCode_${version}_${plat}.tar.gz";
        url = "https://update.code.visualstudio.com/${version}/${plat}/stable";
        inherit hash;
      };
    }
  )).fhs)

  zed-editor

  kdePackages.filelight

  polkit

  pwvucontrol

  kdePackages.qtwayland
  kdePackages.qtsvg
]
