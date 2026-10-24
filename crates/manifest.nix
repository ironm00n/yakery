# The workspace's members as their manifests declare them, and the rules every member manifest keeps.
{ lib }:
let
  workspace = (lib.importTOML ./Cargo.toml).workspace;
  manifest = dir: lib.importTOML (./. + "/${dir}/Cargo.toml");
  tables = [
    "dependencies"
    "build-dependencies"
    "dev-dependencies"
  ];
  deps = table: m: lib.mapAttrsToList (name: spec: { inherit name spec; }) (m.${table} or { });
  allDeps = m: lib.concatMap (table: deps table m) tables;
  local = dep: workspace.dependencies.${dep.name}.path or null;
  # lib/* crates stay usable from the static slurm-ci and from any daemon: no runtime, nothing that calls NSS
  libAllowed = [
    "anyhow"
    "base16ct"
    "getrandom"
    "rustix"
    "serde"
    "serde_json"
    "subtle"
    "text"
    "private"
    "yakery-log"
  ];
  # every package's build reads every member manifest but only its own closure's sources, so this one holds for all members
  targets = _: m: [
    {
      broken =
        !(m ? lib || m ? bin)
        || m ? lib && !(m.lib ? path)
        || lib.any (kind: lib.any (target: !(target ? path)) (m.${kind} or [ ])) [
          "bin"
          "test"
          "example"
          "bench"
        ];
      why = "a target without an explicit path, which cargo cannot find in a build that leaves its sources out";
    }
  ];
  broken =
    rulesOf: dirs:
    lib.concatMap (
      dir:
      map (rule: "crates/${dir}: ${rule.why}") (
        lib.filter (rule: rule.broken) (rulesOf dir (manifest dir))
      )
    ) dirs;
  rules =
    dir: m:
    [
      {
        broken = m.package.edition or null != { workspace = true; };
        why = "edition is not edition.workspace = true";
      }
      {
        broken = m.package.rust-version or null != { workspace = true; };
        why = "rust-version is not rust-version.workspace = true";
      }
      {
        broken = m.lints or null != { workspace = true; };
        why = "[lints] is not workspace = true";
      }
      {
        broken = m ? target;
        why = "a target-specific dependency table, which the workspace rules do not read";
      }
    ]
    ++ map (dep: {
      broken =
        !lib.isAttrs dep.spec
        || dep.spec.workspace or false != true
        ||
          lib.attrNames (
            removeAttrs dep.spec [
              "workspace"
              "features"
              "optional"
            ]
          ) != [ ];
      why = "${dep.name} is not `workspace = true` with at most features or optional";
    }) (allDeps m)
    ++ lib.optionals (lib.hasPrefix "lib/" dir) (
      map (dep: {
        broken = !lib.elem dep.name libAllowed;
        why = "${dep.name} is not on the lib/* allowlist";
      }) (deps "dependencies" m ++ deps "build-dependencies" m)
    );
in
rec {
  inherit manifest;
  inherit (workspace) members;
  dirOf = lib.listToAttrs (map (dir: lib.nameValuePair (manifest dir).package.name dir) members);
  # a member's dir and those of the members it reaches through path dependencies, tests' included
  closure =
    crate:
    map (e: e.key) (
      builtins.genericClosure {
        startSet = [ { key = dirOf.${crate}; } ];
        operator = e: map (key: { inherit key; }) (lib.remove null (map local (allDeps (manifest e.key))));
      }
    );
  # the rules over `dirs`, and explicit target paths over every member
  violations = dirs: broken rules dirs ++ broken targets workspace.members;
  checked = dirs: lib.throwIf (violations dirs != [ ]) (lib.concatLines (violations dirs)) dirs;
}
