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
  vscode.fhs

  zed-editor

  kdePackages.filelight

  polkit

  pwvucontrol

  kdePackages.qtwayland
  kdePackages.qtsvg
]
