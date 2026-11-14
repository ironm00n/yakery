# Its git takes the shares ../shares.nix binds, and the memory dir of its own clone of each project in `memories` is
# the bound one.
{ config, lib, ... }:
let
  inherit (import ../layout.nix { inherit lib; })
    top
    projects
    memories
    memory
    ;
  home = config.home.homeDirectory;
in
{
  # git refuses a tree another uid owns ("dubious ownership"), and each share is his
  programs.git.settings.safe.directory = map (project: "${top}/${project}") projects;
  home.file = lib.listToAttrs (
    map (
      project:
      lib.nameValuePair (lib.removePrefix "${home}/" (memory home project)) {
        source = config.lib.file.mkOutOfStoreSymlink (memory top project);
      }
    ) memories
  );
}
