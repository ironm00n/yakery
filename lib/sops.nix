{ lib }:
let
  mkAttrName =
    {
      prefix,
      separator,
      key,
    }:
    if prefix == "" then key else "${prefix}${separator}${key}";

  normalizeEntry = entry: if builtins.isString entry then { key = entry; } else entry;
in
{
  /**
    Build a set of sops.secrets entries that share a common sopsFile and optional name prefix.

    Each entry be one of:
    - a string representing the sops key
    - an attrset of { key, usergroup?, owner?, group?, sopsFile? }
    where
    - `usergroup` is shorthand that sets both owner and group
    - each key overrides their corresponding default as applicable

    Returns an attrs set { secrets, get-path } where:
    - secrets       : attrs to assign to `sops.secrets`
    - get-path key  : returns config.sops.secrets.<full-name>.path
  */
  mkSecrets =
    {
      config,
      sopsFile,
      prefix ? "",
      separator ? "_",
      usergroup ? null,
      owner ? usergroup,
      group ? usergroup,
    }:
    entries:
    let
      mkOne =
        rawEntry:
        let
          e = normalizeEntry rawEntry;
          ownerDefault = e.usergroup or owner;
          groupDefault = e.usergroup or group;
        in
        {
          name = mkAttrName {
            inherit prefix separator;
            inherit (e) key;
          };
          value = {
            sopsFile = e.sopsFile or sopsFile;
            inherit (e) key;
            owner = e.owner or ownerDefault;
            group = e.group or groupDefault;
          };
        };
    in
    {
      secrets = builtins.listToAttrs (map mkOne entries);
      get-path =
        key:
        config.sops.secrets.${
          mkAttrName {
            inherit prefix separator;
            inherit key;
          }
        }.path;
    };
}
