# identity.test.sh against the agent identity his account's claude bundle generates
{ pkgs, home }:
let
  bundle = ../../users/ironmoon/bundles/dev/claude;
  generated = name: pkgs.writeText name home.xdg.configFile."git/${name}".text;
  config = pkgs.linkFarm "claude-identity-config" {
    "jj/conf.d/claude.toml" = "${bundle}/jj.toml";
    "git/claude-identity" = generated "claude-identity";
    "git/claude-identity.user" = generated "claude-identity.user";
  };
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
