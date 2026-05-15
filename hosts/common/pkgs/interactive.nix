{ pkgs }:

with pkgs;
[
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

  # HACK: needed bc of https://github.com/nix-community/home-manager/issues/5559
  bitwarden-desktop
]
