{ pkgs }:

with pkgs;
[
  qemu

  exiftool

  usbmuxd

  wl-clipboard
  git-filter-repo
  rustup

  os-prober

  gef
  # pwndbg

  emacs-gtk
  vscode.fhs # use the built-in settings sync
  zed-editor

  kdePackages.filelight

  polkit

  pwvucontrol

  kdePackages.qtwayland
  kdePackages.qtsvg
]
