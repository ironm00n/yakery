# FIXME: xb builds with single-user nix, whose build root belongs to the builder, so Forgejo's
# TestCannotCreatePrivateKey finds `/` writable and fails; drop this once xb's build root is read-only
{ pkgs, lib, ... }:
{
  services.forgejo.package = lib.mkOverride 60 (
    (pkgs.callPackage ../../packages/forgejo { }).overrideAttrs (old: {
      checkFlags = map (
        flag: if lib.hasPrefix "-skip=" flag then "${flag}|^TestCannotCreatePrivateKey$" else flag
      ) old.checkFlags;
      passthru = old.passthru // {
        vmTestOnly = true;
      };
    })
  );
}
