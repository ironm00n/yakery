# Its commits name the agent identity and are signed with its own OpenPGP key, made on the host by the account as its
# ssh key is, for GitHub's ironmoon-agent and Forgejo's agent user.
{
  config,
  lib,
  pkgs,
  my-lib,
  ...
}:
let
  agent = config.users.users.claude-agent;
  inherit (my-lib.ironmoon.mail.agents) identity;
  inherit (identity) email;
  gpg = lib.getExe' pkgs.gnupg "gpg";
  gpgconf = lib.getExe' pkgs.gnupg "gpgconf";
in
{
  # ed25519: GnuPG 2.4 makes it a v4 key, which GitHub and Forgejo both read
  systemd.services.claude-agent-gpg-key = {
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      User = agent.name;
    };
    # its journal ends with the public half, for adding it where the agent pushes
    script = ''
      ${gpg} --batch --list-secret-keys ${email} >/dev/null 2>&1 ||
        ${gpg} --batch --pinentry-mode loopback --passphrase "" --quick-gen-key "${identity.name} <${email}>" ed25519 sign never
      ${gpg} --batch --armor --export ${email}
      # a keyboxd left in this unit holds the keyring's lock, under a socket the account's own gpg may not look in
      ${gpgconf} --kill all
    '';
  };

  home-manager.users.claude-agent = {
    programs.git = {
      enable = true;
      settings.user = identity;
      signing = {
        key = email;
        signByDefault = true;
        format = "openpgp";
        signer = gpg;
      };
    };
    programs.jujutsu = {
      enable = true;
      settings = {
        user = identity;
        signing = {
          behavior = "own";
          backend = "gpg";
          key = email;
          backends.gpg.program = gpg;
        };
      };
    };
  };
}
