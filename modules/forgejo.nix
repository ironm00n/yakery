# nixpkgs' Forgejo module, edited so that from Forgejo 18, which no longer reads INTERNAL_TOKEN
# or can generate one, it neither declares nor generates it, and a fresh state dir starts
{ lib, modulesPath, ... }:
let
  file = "services/misc/forgejo.nix";
  lines = lib.concatMapStrings (line: line + "\n");
  around =
    text:
    {
      before,
      after ? "",
    }:
    {
      from = text;
      to = before + text + after;
    };
  edit =
    text:
    { from, to }:
    assert lib.assertMsg (
      lib.length (lib.splitString from text) == 2
    ) "nixpkgs' ${file} no longer holds exactly one copy of a text modules/forgejo.nix edits";
    lib.replaceStrings [ from ] [ to ] text;
  token = "'\${cfg.secrets.security.INTERNAL_TOKEN}'";
in
{
  disabledModules = [ file ];
  # imported as a function, so the module system takes it as declared here: the manual names a module's chapter after
  # its file, and the copy's store name isn't a valid id
  imports = [
    (import (
      builtins.toFile "forgejo.nix" (
        lib.foldl edit (builtins.readFile "${modulesPath}/${file}") [
          (around (lines [ "        INTERNAL_TOKEN = \"\${cfg.customDir}/conf/internal_token\";" ]) {
            before = lines [
              "      }"
              "      // optionalAttrs (lib.versionOlder cfg.package.version \"18\") {"
            ];
          })
          (around
            (lines [
              "        if [ ! -s ${token} ]; then"
              "            \${exe} generate secret INTERNAL_TOKEN > ${token}"
              "        fi"
            ])
            {
              before = lines [ "        \${optionalString (cfg.secrets.security ? INTERNAL_TOKEN) ''" ];
              after = lines [ "        ''}" ];
            }
          )
          {
            from = "meta.doc = ./forgejo.md;";
            to = "meta.doc = ${modulesPath}/${dirOf file}/forgejo.md;";
          }
        ]
      )
    ))
  ];
}
