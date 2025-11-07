{ pkgs }:

with pkgs;
[
  # GNU
  coreutils-full
  findutils
  diffutils
  gawk
  gnused
  gnugrep
  binutils
  gnupg
  gnumake

  util-linux
  moreutils
  psmisc
  hexedit
  tree
  ltrace
  strace
  pv

  git
  htop
  btop
  file

  ripgrep
  fd
  delta
  dust
  moor # much nicer pager

  gcc
  gdb
  valgrind

  zsh
  neovim
  helix
  gh

  inotify-tools

  man-pages
  tldr

  docker

  neofetch
  hyfetch

  zip
  unzip

  cachix

  fastfetch
]
