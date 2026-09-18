{ ... }:
{
  projectRootFile = "flake.nix";
  settings.global.excludes = [
    ".envrc"
    "*.opam"
    "*.zip"
    "*.tar.gz"
    "users/ironmoon/bundles/nvim/highlight.nix"
    "flake.nix"
    # TODO: update to perminant location
    "hosts/fw13/experiments/stalwart-roles.nix"
  ];
  programs = {
    deadnix.enable = false;
    nixfmt.enable = true;
    jsonfmt.enable = true;
    # mdformat.enable = true;
  };
}
