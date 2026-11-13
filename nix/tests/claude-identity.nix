# identity.test.sh against the agent identity his account's claude bundle generates
{ pkgs, home }:
let
  bundle = ../../users/ironmoon/bundles/dev/claude;
  generated = name: pkgs.writeText name home.xdg.configFile."git/${name}".text;
  # the host's other conf.d files beside claude.toml, whose order decides which key signs
  hostConfD = pkgs.lib.mapAttrs (_: f: f.source) (
    pkgs.lib.filterAttrs (
      name: _: pkgs.lib.hasPrefix "jj/conf.d/" name && name != "jj/conf.d/claude.toml"
    ) home.xdg.configFile
  );
  config = pkgs.linkFarm "claude-identity-config" (
    {
      "jj/conf.d/claude.toml" = "${bundle}/jj.toml";
      "git/claude-identity" = generated "claude-identity";
      "git/claude-identity.user" = generated "claude-identity.user";
    }
    // hostConfD
  );
in
pkgs.runCommand "claude-identity"
  {
    nativeBuildInputs = [
      pkgs.jujutsu
      pkgs.git
      pkgs.jq
    ];
  }
  ''
    HOME=$TMPDIR bash ${bundle}/identity.test.sh ${config} ${bundle}/settings.json
    touch $out
  ''
