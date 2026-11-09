# Every account's commits name the agent identity and are signed with one OpenPGP key, for GitHub's ironmoon-agent and
# Forgejo's agent user: the one he copies into all of them with agent-adopt-gpg-key. Until then each signs with a key it
# makes on the host as it does its ssh key.
{
  lib,
  pkgs,
  my-lib,
  ...
}:
let
  inherit (my-lib.ironmoon.mail.agents) identity;
  inherit (identity) email;
  gpg = lib.getExe' pkgs.gnupg "gpg";
  accounts = builtins.attrValues (import ./accounts.nix);
  adopt = "agent-adopt-gpg-key";
  account =
    { name, uid, ... }:
    let
      userManager = "user@${toString uid}.service";
    in
    {
      # ed25519: GnuPG 2.4 makes it a v4 key, which GitHub and Forgejo both read
      systemd.services."${name}-gpg-key" = {
        wantedBy = [ "multi-user.target" ];
        # its user manager holds the account's gpg-agent socket; a gpg run before that starts its own agent, which exits 2
        wants = [ userManager ];
        after = [ userManager ];
        # not RemainAfterExit, so the keyboxd its gpg starts ends with it
        serviceConfig = {
          Type = "oneshot";
          User = name;
        };
        # its journal ends with the public half, for adding it where the agent pushes
        script = ''
          set -o pipefail
          ${gpg} --batch --list-secret-keys ${email} >/dev/null 2>&1 ||
            ${gpg} --batch --pinentry-mode loopback --passphrase "" --quick-gen-key "${identity.name} <${email}>" ed25519 sign never
          # git and jj name the key by its email, and the account holds no key but that one
          keys=$(${gpg} --batch --with-colons --list-secret-keys | grep -c '^sec:')
          [ "$keys" = 1 ] || { echo "$keys secret keys, not ${email}'s alone; ${adopt} leaves one" >&2; exit 1; }
          # the journal drops empty lines, and a whitespace line still ends the armor's headers
          ${gpg} --batch --armor --export ${email} | sed 's/^$/ /'
        '';
      };

      home-manager.users.${name} = {
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
    };
in
{
  config = lib.mkMerge (
    [
      {
        environment.systemPackages = [
          (pkgs.writeShellApplication {
            name = adopt;
            runtimeInputs = [ pkgs.gnupg ];
            runtimeEnv = {
              inherit email;
              accounts = map (a: a.name) accounts;
            };
            text = builtins.readFile ./adopt-gpg-key.sh;
          })
        ];
      }
    ]
    ++ map account accounts
  );
}
