{ pkgs }:

with pkgs;
[
  dmidecode
  os-prober

  exiftool

  usbmuxd

  wl-clipboard
  git-filter-repo
  rustup

  kitty
  emacs-gtk
  vscode.fhs # use the built-in settings sync
  zed-editor

  kdePackages.filelight

  polkit

  pwvucontrol

  kdePackages.qtwayland
  kdePackages.qtsvg
]
