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
  file

  ripgrep
  fd
  delta
  dust
  moar # much nicer pager

  gcc
  gdb
  valgrind

  zsh
  neovim
  helix
  gh

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
